//! A minimal SMTP server for tests: accepts mail, optionally requires `AUTH PLAIN`, records what it
//! received. Plain text only (no STARTTLS). Not a general SMTP implementation.

use std::sync::{Arc, Mutex};

use base64::Engine;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Received {
    pub from: String,
    pub to: Vec<String>,
    pub data: String,
    pub auth_user: Option<String>,
}

#[derive(Clone, Default)]
pub struct Behavior {
    /// `(user, password)` required via `AUTH PLAIN` before `MAIL FROM`.
    pub require_auth: Option<(String, String)>,
    /// Reject every recipient with `550`.
    pub reject_recipients: bool,
}

pub struct TestSmtp {
    pub port: u16,
    mails: Arc<Mutex<Vec<Received>>>,
    task: JoinHandle<()>,
}

impl Drop for TestSmtp {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl TestSmtp {
    pub async fn spawn(behavior: Behavior) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test smtp");
        let port = listener.local_addr().expect("local addr").port();
        let mails = Arc::new(Mutex::new(Vec::new()));
        let store = Arc::clone(&mails);
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let (store, behavior) = (Arc::clone(&store), behavior.clone());
                tokio::spawn(async move {
                    let _ = serve(stream, store, behavior).await;
                });
            }
        });
        Self { port, mails, task }
    }

    pub fn received(&self) -> Vec<Received> {
        self.mails.lock().map(|m| m.clone()).unwrap_or_default()
    }
}

fn address(arg: &str) -> String {
    arg.split_once(':')
        .map_or(arg, |(_, rest)| rest)
        .trim()
        .trim_matches(|c| c == '<' || c == '>')
        .to_owned()
}

async fn serve(
    stream: tokio::net::TcpStream,
    store: Arc<Mutex<Vec<Received>>>,
    behavior: Behavior,
) -> std::io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    write.write_all(b"220 test ESMTP\r\n").await?;
    let mut current = Received::default();
    let mut authenticated: Option<String> = None;
    while let Some(line) = lines.next_line().await? {
        let upper = line.to_ascii_uppercase();
        if upper.starts_with("EHLO") || upper.starts_with("HELO") {
            let auth = if behavior.require_auth.is_some() {
                "250-AUTH PLAIN\r\n"
            } else {
                ""
            };
            write
                .write_all(format!("250-test\r\n{auth}250 8BITMIME\r\n").as_bytes())
                .await?;
        } else if upper.starts_with("AUTH PLAIN") {
            let token = line.split_whitespace().nth(2).unwrap_or_default();
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(token)
                .unwrap_or_default();
            let parts: Vec<&[u8]> = decoded.split(|b| *b == 0).collect();
            let given = (
                String::from_utf8_lossy(parts.get(1).copied().unwrap_or_default()).into_owned(),
                String::from_utf8_lossy(parts.get(2).copied().unwrap_or_default()).into_owned(),
            );
            if behavior.require_auth.as_ref() == Some(&given) {
                authenticated = Some(given.0);
                write.write_all(b"235 ok\r\n").await?;
            } else {
                write.write_all(b"535 bad credentials\r\n").await?;
            }
        } else if upper.starts_with("MAIL FROM") {
            if behavior.require_auth.is_some() && authenticated.is_none() {
                write.write_all(b"530 authentication required\r\n").await?;
            } else {
                current = Received {
                    from: address(&line[9..]),
                    auth_user: authenticated.clone(),
                    ..Received::default()
                };
                write.write_all(b"250 ok\r\n").await?;
            }
        } else if upper.starts_with("RCPT TO") {
            if behavior.reject_recipients {
                write.write_all(b"550 no such user\r\n").await?;
            } else {
                current.to.push(address(&line[7..]));
                write.write_all(b"250 ok\r\n").await?;
            }
        } else if upper == "DATA" {
            write.write_all(b"354 go ahead\r\n").await?;
            let mut data = String::new();
            while let Some(l) = lines.next_line().await? {
                if l == "." {
                    break;
                }
                data.push_str(l.strip_prefix("..").map_or(&l, |_| &l[1..]));
                data.push('\n');
            }
            current.data = data;
            if let Ok(mut mails) = store.lock() {
                mails.push(current.clone());
            }
            write.write_all(b"250 queued\r\n").await?;
        } else if upper == "RSET" || upper == "NOOP" {
            write.write_all(b"250 ok\r\n").await?;
        } else if upper == "QUIT" {
            write.write_all(b"221 bye\r\n").await?;
            break;
        } else {
            write.write_all(b"502 not implemented\r\n").await?;
        }
    }
    Ok(())
}
