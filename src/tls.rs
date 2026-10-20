//! Built-in HTTPS: a self-signed certificate for the server (`opennetmon tls generate`), loading certificate
//! and key, and the server loop. With TLS configured the port speaks only HTTPS; a plain-HTTP request on it
//! gets a redirect to the same address with `https://`, so nothing is ever served unencrypted.

use std::convert::Infallible;
use std::fs;
use std::future::Future;
use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{HeaderValue, Request, Response, StatusCode, header};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;
use rcgen::{
    CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, KeyPair, KeyUsagePurpose,
};
use sha2::{Digest, Sha256};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::crypto::ring;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tower::ServiceExt;

/// Validity of a generated certificate.
const VALID_DAYS: i64 = 10 * 365;
/// Time a client gets for the first byte and for the TLS handshake.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Time a client gets to send the request headers (against connections that are opened and left idle).
const HEADER_TIMEOUT: Duration = Duration::from_secs(30);
/// Open requests may finish this long after shutdown begins.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(10);
/// First byte of a TLS connection (handshake record).
const TLS_HANDSHAKE: u8 = 0x16;

#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("{path}: {reason}")]
    File { path: PathBuf, reason: String },
    #[error("invalid name for the certificate: '{0}' (expected a host name or an IP address)")]
    Name(String),
    #[error("cannot create the certificate: {0}")]
    Generate(String),
    #[error("certificate and key do not fit together or are not usable: {0}")]
    Config(String),
}

fn file_err(path: &Path, reason: impl Into<String>) -> TlsError {
    TlsError::File {
        path: path.to_owned(),
        reason: reason.into(),
    }
}

/// Result of [`generate`]: the names in the certificate and its SHA-256 fingerprint.
#[derive(Debug)]
pub struct Generated {
    pub names: Vec<String>,
    pub fingerprint: String,
}

/// The names a generated certificate covers by default: the host name, `localhost` and the loopback
/// addresses; `extra` (host names, IP addresses of the server) is added in front, duplicates dropped.
pub fn certificate_names(extra: &[String]) -> Vec<String> {
    let host = fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default();
    let mut names: Vec<String> = Vec::new();
    for name in extra
        .iter()
        .map(|n| n.trim().to_owned())
        .chain([host.trim().to_owned()])
        .chain(["localhost", "127.0.0.1", "::1"].map(str::to_owned))
    {
        if !name.is_empty() && !names.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
            names.push(name);
        }
    }
    names
}

/// A host name (letters, digits and hyphens per label) or an IP address.
pub fn is_valid_name(name: &str) -> bool {
    let host_name = !name.is_empty()
        && name.len() <= 253
        && name.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    host_name || name.parse::<IpAddr>().is_ok()
}

/// Creates a self-signed certificate (ECDSA P-256, valid for ten years) for `names` and writes it to
/// `cert_path` (mode 0644) and its key to `key_path` (mode 0600). Never overwrites an existing file.
pub fn generate(
    cert_path: &Path,
    key_path: &Path,
    names: &[String],
) -> Result<Generated, TlsError> {
    if let Some(name) = names.iter().find(|n| !is_valid_name(n)) {
        return Err(TlsError::Name(name.clone()));
    }
    for path in [cert_path, key_path] {
        if path.exists() {
            return Err(file_err(
                path,
                "already exists (an existing file is never overwritten)",
            ));
        }
    }
    let gen_err = |e: rcgen::Error| TlsError::Generate(e.to_string());
    let mut params = CertificateParams::new(names.to_vec()).map_err(gen_err)?;
    let mut subject = DistinguishedName::new();
    subject.push(
        DnType::CommonName,
        format!("opennetmon {}", names.first().map_or("", String::as_str)),
    );
    params.distinguished_name = subject;
    let now = time::OffsetDateTime::now_utc();
    params.not_before = now - time::Duration::days(1);
    params.not_after = now + time::Duration::days(VALID_DAYS);
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let key = KeyPair::generate().map_err(gen_err)?;
    let cert = params.self_signed(&key).map_err(gen_err)?;

    // The key first: a certificate without its key would block the next attempt.
    write_new(key_path, key.serialize_pem().as_bytes(), 0o600)?;
    write_new(cert_path, cert.pem().as_bytes(), 0o644)?;
    Ok(Generated {
        names: names.to_vec(),
        fingerprint: fingerprint(cert.der()),
    })
}

fn write_new(path: &Path, content: &[u8], mode: u32) -> Result<(), TlsError> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .map_err(|e| file_err(path, format!("cannot create directory: {e}")))?;
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)
        .map_err(|e| {
            file_err(
                path,
                format!("cannot create (an existing file is never overwritten): {e}"),
            )
        })?;
    file.write_all(content)
        .map_err(|e| file_err(path, format!("cannot write: {e}")))
}

/// SHA-256 fingerprint as browsers show it (`AB:CD:…`).
pub fn fingerprint(cert: &CertificateDer<'_>) -> String {
    Sha256::digest(cert.as_ref())
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// A loaded certificate chain with its key, ready for the server.
pub struct Loaded {
    pub config: Arc<ServerConfig>,
    /// Fingerprint of the server certificate (the first one in the chain file).
    pub fingerprint: String,
}

/// Loads the certificate chain (PEM, server certificate first, then intermediates) and the private key
/// (PEM: PKCS#8, PKCS#1 or SEC1). Refuses a key file that group or others can access.
pub fn load(cert_path: &Path, key_path: &Path) -> Result<Loaded, TlsError> {
    let meta =
        fs::metadata(key_path).map_err(|e| file_err(key_path, format!("cannot read: {e}")))?;
    if meta.permissions().mode() & 0o077 != 0 {
        return Err(file_err(
            key_path,
            "must not be accessible by group or others (run `chmod 600`)",
        ));
    }
    let certs = CertificateDer::pem_file_iter(cert_path)
        .map_err(|e| file_err(cert_path, format!("cannot read: {e}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| file_err(cert_path, format!("not a PEM certificate file: {e}")))?;
    let Some(first) = certs.first() else {
        return Err(file_err(cert_path, "contains no certificate"));
    };
    let fingerprint = fingerprint(first);
    let key = PrivateKeyDer::from_pem_file(key_path)
        .map_err(|e| file_err(key_path, format!("not a PEM private key: {e}")))?;
    let mut config = ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| TlsError::Config(e.to_string()))?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| TlsError::Config(e.to_string()))?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Loaded {
        config: Arc::new(config),
        fingerprint,
    })
}

/// Serves `app` over HTTPS until `shutdown` completes, then lets open requests finish (up to 10 s).
/// The handlers see the client address as `ConnectInfo<SocketAddr>`, as with `axum::serve`.
pub async fn serve(
    listener: TcpListener,
    app: Router,
    tls: Arc<ServerConfig>,
    shutdown: impl Future<Output = ()>,
) {
    let acceptor = TlsAcceptor::from(tls);
    let graceful = GracefulShutdown::new();
    tokio::pin!(shutdown);
    loop {
        let accepted = tokio::select! {
            accepted = listener.accept() => accepted,
            () = &mut shutdown => break,
        };
        let (stream, peer) = match accepted {
            Ok(connection) => connection,
            Err(e) => {
                // e.g. too many open files: wait instead of spinning, like axum::serve.
                tracing::warn!(error = %e, "accepting a connection failed");
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        let acceptor = acceptor.clone();
        let app = app.clone();
        let watcher = graceful.watcher();
        tokio::spawn(async move {
            handle(stream, peer, acceptor, app, watcher).await;
        });
    }
    drop(listener);
    if tokio::time::timeout(SHUTDOWN_GRACE, graceful.shutdown())
        .await
        .is_err()
    {
        tracing::warn!("open connections did not finish in time");
    }
}

async fn handle(
    stream: TcpStream,
    peer: SocketAddr,
    acceptor: TlsAcceptor,
    app: Router,
    watcher: hyper_util::server::graceful::Watcher,
) {
    let mut first = [0u8; 1];
    match tokio::time::timeout(HANDSHAKE_TIMEOUT, stream.peek(&mut first)).await {
        Ok(Ok(1)) => {}
        _ => return,
    }
    let mut builder = http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(HEADER_TIMEOUT);

    if first[0] != TLS_HANDSHAKE {
        let service = hyper::service::service_fn(|request| async move {
            Ok::<_, Infallible>(redirect_to_https(&request))
        });
        let connection = builder.serve_connection(TokioIo::new(stream), service);
        if let Err(e) = watcher.watch(connection).await {
            tracing::debug!(%peer, error = %e, "plain-HTTP connection ended with an error");
        }
        return;
    }

    let stream = match tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(stream)).await {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => {
            // Usually a browser that does not (yet) trust the certificate.
            tracing::debug!(%peer, error = %e, "TLS handshake failed");
            return;
        }
        Err(_) => return,
    };
    let service = app.map_request(move |mut request: Request<Incoming>| {
        request.extensions_mut().insert(ConnectInfo(peer));
        request
    });
    let connection =
        builder.serve_connection(TokioIo::new(stream), TowerToHyperService::new(service));
    if let Err(e) = watcher.watch(connection).await {
        tracing::debug!(%peer, error = %e, "HTTPS connection ended with an error");
    }
}

/// `301` to the same host, port, path and query with `https://`; `400` without a usable `Host` header.
fn redirect_to_https<B>(request: &Request<B>) -> Response<Body> {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .filter(|h| {
            !h.is_empty()
                && h.len() <= 255
                && h.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b))
        });
    let path = request.uri().path_and_query().map_or("/", |p| p.as_str());
    let location =
        host.and_then(|host| HeaderValue::from_str(&format!("https://{host}{path}")).ok());
    let mut response = Response::new(Body::from(match location {
        Some(_) => "This server only speaks HTTPS.\n",
        None => "This server only speaks HTTPS: open it with https://\n",
    }));
    match location {
        Some(location) => {
            *response.status_mut() = StatusCode::MOVED_PERMANENTLY;
            response.headers_mut().insert(header::LOCATION, location);
        }
        None => *response.status_mut() = StatusCode::BAD_REQUEST,
    }
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(header::CONNECTION, HeaderValue::from_static("close"));
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_rustls::TlsConnector;
    use tokio_rustls::rustls::pki_types::ServerName;
    use tokio_rustls::rustls::{ClientConfig, RootCertStore};

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn generated_files_have_tight_modes_and_are_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let (cert, key) = (
            dir.path().join("tls/cert.pem"),
            dir.path().join("tls/key.pem"),
        );
        let generated = generate(&cert, &key, &names(&["server.lan", "192.168.0.10"])).unwrap();
        assert_eq!(generated.names, ["server.lan", "192.168.0.10"]);
        assert_eq!(generated.fingerprint.len(), 32 * 3 - 1);
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!((mode(&cert), mode(&key)), (0o644, 0o600));
        let loaded = load(&cert, &key).unwrap();
        assert_eq!(loaded.fingerprint, generated.fingerprint);

        let before = fs::read(&key).unwrap();
        let again = generate(&cert, &key, &names(&["server.lan"])).unwrap_err();
        assert!(again.to_string().contains("never overwritten"), "{again}");
        assert_eq!(fs::read(&key).unwrap(), before);
    }

    #[test]
    fn names_are_checked_and_the_defaults_added_once() {
        let dir = tempfile::tempdir().unwrap();
        for bad in ["", "a b", "x/y", "-lead.lan", "café.lan"] {
            let err = generate(&dir.path().join("c"), &dir.path().join("k"), &names(&[bad]));
            assert!(matches!(err, Err(TlsError::Name(_))), "{bad}");
        }
        let all = certificate_names(&names(&["Server.lan", "10.0.0.5", "localhost", " "]));
        assert_eq!(&all[..2], ["Server.lan", "10.0.0.5"]);
        assert!(all.ends_with(&names(&["127.0.0.1", "::1"])));
        assert_eq!(all.iter().filter(|n| *n == "localhost").count(), 1);
        assert_eq!(
            certificate_names(&[]).last().map(String::as_str),
            Some("::1")
        );
    }

    #[test]
    fn a_readable_key_or_a_wrong_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (cert, key) = (dir.path().join("cert.pem"), dir.path().join("key.pem"));
        generate(&cert, &key, &names(&["localhost"])).unwrap();
        fs::set_permissions(&key, fs::Permissions::from_mode(0o640)).unwrap();
        let err = load(&cert, &key).err().unwrap();
        assert!(err.to_string().contains("chmod 600"), "{err}");
        fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
        // Certificate and key swapped.
        let err = load(&key, &key).err().unwrap();
        assert!(err.to_string().contains("no certificate"), "{err}");
    }

    #[test]
    fn plain_http_is_redirected_to_https() {
        let request = Request::builder()
            .uri("/sensors/3?range=24h")
            .header(header::HOST, "192.168.0.10:8443")
            .body(())
            .unwrap();
        let response = redirect_to_https(&request);
        assert_eq!(response.status(), StatusCode::MOVED_PERMANENTLY);
        assert_eq!(
            response.headers()[header::LOCATION],
            "https://192.168.0.10:8443/sensors/3?range=24h"
        );
        for host in [None, Some("evil.example/x"), Some("")] {
            let mut builder = Request::builder().uri("/");
            if let Some(host) = host {
                builder = builder.header(header::HOST, host);
            }
            let response = redirect_to_https(&builder.body(()).unwrap());
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{host:?}");
            assert!(response.headers().get(header::LOCATION).is_none());
        }
    }

    async fn read_all(mut stream: impl tokio::io::AsyncRead + Unpin) -> String {
        let mut out = Vec::new();
        let _ = stream.read_to_end(&mut out).await;
        String::from_utf8_lossy(&out).into_owned()
    }

    #[tokio::test]
    async fn serves_https_and_redirects_plain_http_on_the_same_port() {
        let dir = tempfile::tempdir().unwrap();
        let (cert, key) = (dir.path().join("cert.pem"), dir.path().join("key.pem"));
        generate(&cert, &key, &names(&["localhost"])).unwrap();
        let loaded = load(&cert, &key).unwrap();
        let app = Router::new().route(
            "/peer",
            get(|ConnectInfo(peer): ConnectInfo<SocketAddr>| async move { peer.ip().to_string() }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(serve(listener, app, loaded.config, async {
            let _ = stopped.await;
        }));

        // HTTPS with the generated certificate as the only trusted root.
        let mut roots = RootCertStore::empty();
        for c in CertificateDer::pem_file_iter(&cert).unwrap() {
            roots.add(c.unwrap()).unwrap();
        }
        let client = ClientConfig::builder_with_provider(Arc::new(ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let tcp = TcpStream::connect(addr).await.unwrap();
        let mut tls = TlsConnector::from(Arc::new(client))
            .connect(ServerName::try_from("localhost").unwrap(), tcp)
            .await
            .unwrap();
        tls.write_all(b"GET /peer HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let response = read_all(tls).await;
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.ends_with("127.0.0.1"), "{response}");

        // Plain HTTP on the same port: only a redirect, never the page.
        let mut plain = TcpStream::connect(addr).await.unwrap();
        let request = format!(
            "GET /peer HTTP/1.1\r\nHost: localhost:{}\r\n\r\n",
            addr.port()
        );
        plain.write_all(request.as_bytes()).await.unwrap();
        let response = read_all(plain).await;
        assert!(response.starts_with("HTTP/1.1 301"), "{response}");
        assert!(
            response.contains(&format!("location: https://localhost:{}/peer", addr.port())),
            "{response}"
        );
        assert!(!response.contains("127.0.0.1"));

        stop.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
    }
}
