//! A small HTTP(S) client for sensors (and later notifications and the update check): one request
//! per connection, HTTP/1.1, rustls with the Mozilla roots of `webpki-roots`, optional redirects,
//! a cap on the body that is read and one deadline for everything.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::{Method, Request, StatusCode, Uri, header};
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::client::WebPkiServerVerifier;
use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::crypto::ring;
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio_rustls::rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
};

/// Longest accepted URL.
pub const MAX_URL_CHARS: usize = 2_000;
/// Redirects followed at most.
pub const MAX_REDIRECTS: usize = 5;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HttpError {
    #[error("invalid URL")]
    InvalidUrl,
    #[error("timed out")]
    Timeout,
    #[error("cannot connect: {0}")]
    Connect(String),
    #[error("TLS: {0}")]
    Tls(String),
    #[error("HTTP: {0}")]
    Protocol(String),
    #[error("more than {MAX_REDIRECTS} redirects")]
    TooManyRedirects,
}

#[derive(Debug, Clone)]
pub struct Options {
    /// `false` sends HEAD instead of GET.
    pub get: bool,
    pub follow_redirects: bool,
    /// Check the certificate chain and name (`false` accepts any certificate, e.g. a self-signed one).
    pub verify_tls: bool,
    /// Bytes of the body that are kept at most; the rest is not read.
    pub max_body: usize,
    /// Deadline for the whole request, redirects included.
    pub timeout: Duration,
}

#[derive(Debug)]
pub struct Response {
    pub status: StatusCode,
    /// The first `max_body` bytes of the body.
    pub body: Vec<u8>,
    pub headers: header::HeaderMap,
    /// Time until the response head of the last request arrived.
    pub elapsed: Duration,
}

/// One request to the API of a device: own method, headers and body, no redirects.
#[derive(Debug, Clone)]
pub struct ApiCall<'a> {
    pub method: Method,
    pub url: &'a str,
    pub headers: &'a [(&'a str, &'a str)],
    pub body: &'a str,
    /// Bytes of the response body that are kept at most; the rest is not read.
    pub max_body: usize,
    /// Deadline for the whole request.
    pub timeout: Duration,
}

/// What goes out on one connection.
struct Outgoing<'a> {
    method: Method,
    headers: &'a [(&'a str, &'a str)],
    body: Bytes,
    max_body: usize,
    verify_tls: bool,
}

/// Where a URL points to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub tls: bool,
    /// Host name or IP address, without brackets.
    pub host: String,
    pub port: u16,
    /// Path and query, at least `/`.
    pub path: String,
    /// What the `Host` header says (`host` or `host:port`).
    pub authority: String,
}

/// Parses an absolute `http://` or `https://` URL. User names and passwords in the URL are refused,
/// because the URL is stored and shown as it is.
pub fn parse_url(url: &str) -> Result<Target, HttpError> {
    if url.chars().count() > MAX_URL_CHARS || url.chars().any(|c| c.is_control() || c == ' ') {
        return Err(HttpError::InvalidUrl);
    }
    let uri: Uri = url.parse().map_err(|_| HttpError::InvalidUrl)?;
    let tls = match uri.scheme_str() {
        Some("https") => true,
        Some("http") => false,
        _ => return Err(HttpError::InvalidUrl),
    };
    let authority = uri.authority().ok_or(HttpError::InvalidUrl)?;
    if authority.as_str().contains('@') {
        return Err(HttpError::InvalidUrl);
    }
    let host = authority
        .host()
        .trim_start_matches('[')
        .trim_end_matches(']');
    if host.is_empty() {
        return Err(HttpError::InvalidUrl);
    }
    let port = authority.port_u16().unwrap_or(if tls { 443 } else { 80 });
    let path = uri
        .path_and_query()
        .map(|p| p.as_str())
        .filter(|p| !p.is_empty())
        .unwrap_or("/");
    Ok(Target {
        tls,
        host: host.to_owned(),
        port,
        path: path.to_owned(),
        authority: authority.as_str().to_owned(),
    })
}

/// The target a `Location` header of a response to `from` points to.
fn redirect_target(from: &Target, location: &str) -> Result<Target, HttpError> {
    let location = location.trim();
    if location.starts_with("http://") || location.starts_with("https://") {
        return parse_url(location);
    }
    let scheme = if from.tls { "https" } else { "http" };
    if let Some(rest) = location.strip_prefix("//") {
        return parse_url(&format!("{scheme}://{rest}"));
    }
    let path = if location.starts_with('/') {
        location.to_owned()
    } else {
        // Relative to the directory of the current path.
        let current = from.path.split('?').next().unwrap_or("/");
        let dir = &current[..current.rfind('/').map_or(0, |i| i + 1)];
        format!("{}{location}", if dir.is_empty() { "/" } else { dir })
    };
    parse_url(&format!("{scheme}://{}{path}", from.authority))
}

/// Sends one request to `url`, following redirects if asked to.
pub async fn fetch(url: &str, options: &Options) -> Result<Response, HttpError> {
    let target = parse_url(url)?;
    tokio::time::timeout(options.timeout, fetch_from(target, options))
        .await
        .unwrap_or(Err(HttpError::Timeout))
}

async fn fetch_from(mut target: Target, options: &Options) -> Result<Response, HttpError> {
    let mut method = if options.get {
        Method::GET
    } else {
        Method::HEAD
    };
    for _ in 0..=MAX_REDIRECTS {
        let outgoing = Outgoing {
            method: method.clone(),
            headers: &[],
            body: Bytes::new(),
            max_body: options.max_body,
            verify_tls: options.verify_tls,
        };
        let response = request_once(&target, &outgoing).await?;
        let location = response
            .headers
            .get(header::LOCATION)
            .and_then(|l| l.to_str().ok());
        match location {
            Some(location) if options.follow_redirects && response.status.is_redirection() => {
                if response.status == StatusCode::SEE_OTHER && method != Method::HEAD {
                    method = Method::GET;
                }
                target = redirect_target(&target, location)?;
            }
            _ => return Ok(response),
        }
    }
    Err(HttpError::TooManyRedirects)
}

/// Sends one request to the API of a device. Certificates are not checked: devices present their
/// own (the request goes to an address the administrator entered).
pub async fn call(request: &ApiCall<'_>) -> Result<Response, HttpError> {
    let target = parse_url(request.url)?;
    let outgoing = Outgoing {
        method: request.method.clone(),
        headers: request.headers,
        body: Bytes::copy_from_slice(request.body.as_bytes()),
        max_body: request.max_body,
        verify_tls: false,
    };
    tokio::time::timeout(request.timeout, request_once(&target, &outgoing))
        .await
        .unwrap_or(Err(HttpError::Timeout))
}

async fn request_once(target: &Target, outgoing: &Outgoing<'_>) -> Result<Response, HttpError> {
    let started = Instant::now();
    let tcp = connect_tcp(&target.host, target.port).await?;
    if target.tls {
        let (tls, _) = connect_tls(tcp, &target.host, outgoing.verify_tls, false).await?;
        exchange(tls, target, outgoing, started).await
    } else {
        exchange(tcp, target, outgoing, started).await
    }
}

/// Opens a TCP connection (all addresses of the name in turn, as `getaddrinfo` returns them).
pub async fn connect_tcp(host: &str, port: u16) -> Result<TcpStream, HttpError> {
    TcpStream::connect((host, port))
        .await
        .map_err(|e| HttpError::Connect(e.to_string()))
}

/// Aborts the connection task when the request is done or given up.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn exchange<S>(
    stream: S,
    target: &Target,
    outgoing: &Outgoing<'_>,
    started: Instant,
) -> Result<Response, HttpError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let protocol = |e: hyper::Error| HttpError::Protocol(e.to_string());
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .map_err(protocol)?;
    let _connection = AbortOnDrop(tokio::spawn(async move {
        let _ = connection.await;
    }));
    let mut request = Request::builder()
        .method(outgoing.method.clone())
        .uri(target.path.as_str())
        .header(header::HOST, target.authority.as_str())
        .header(
            header::USER_AGENT,
            concat!("opennetmon/", env!("CARGO_PKG_VERSION")),
        )
        .header(header::ACCEPT, "*/*")
        .header(header::CONNECTION, "close");
    for (name, value) in outgoing.headers {
        request = request.header(*name, *value);
    }
    let request = request
        .body(Full::new(outgoing.body.clone()))
        .map_err(|_| HttpError::InvalidUrl)?;
    let response = sender.send_request(request).await.map_err(protocol)?;
    let elapsed = started.elapsed();
    let (head, mut body) = response.into_parts();
    let mut kept = Vec::new();
    while kept.len() < outgoing.max_body {
        match body.frame().await {
            Some(Ok(frame)) => {
                if let Ok(data) = frame.into_data() {
                    let room = outgoing.max_body - kept.len();
                    kept.extend_from_slice(&data[..data.len().min(room)]);
                }
            }
            Some(Err(e)) => return Err(protocol(e)),
            None => break,
        }
    }
    Ok(Response {
        status: head.status,
        body: kept,
        headers: head.headers,
        elapsed,
    })
}

/// What the TLS handshake found out about the server's certificate.
#[derive(Debug, Clone)]
pub struct PeerCertificate {
    /// The server's own certificate (not the chain).
    pub der: CertificateDer<'static>,
    /// Why the chain or the name is not trusted; `None` = trusted. An expired certificate counts as
    /// trusted here (see [`PeerCertificate::expired`]), so the days until expiry can still be shown.
    pub problem: Option<String>,
    pub expired: bool,
}

/// Mozilla's root certificates, loaded once.
fn web_pki_verifier() -> Result<Arc<WebPkiServerVerifier>, HttpError> {
    static ROOTS: OnceLock<Arc<RootCertStore>> = OnceLock::new();
    let roots = ROOTS
        .get_or_init(|| {
            Arc::new(RootCertStore::from_iter(
                webpki_roots::TLS_SERVER_ROOTS.iter().cloned(),
            ))
        })
        .clone();
    WebPkiServerVerifier::builder_with_provider(roots, Arc::new(ring::default_provider()))
        .build()
        .map_err(|e| HttpError::Tls(e.to_string()))
}

/// Checks the chain with the web PKI and remembers the result. Unless `enforce` is set, the
/// handshake goes on with an untrusted certificate (the signature of the handshake is still checked);
/// with `accept_expired`, an otherwise trusted but expired certificate passes as well.
#[derive(Debug)]
struct Recording {
    inner: Arc<WebPkiServerVerifier>,
    enforce: bool,
    accept_expired: bool,
    seen: Mutex<Option<PeerCertificate>>,
}

impl ServerCertVerifier for Recording {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, tokio_rustls::rustls::Error> {
        use tokio_rustls::rustls::Error;
        let checked = self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        );
        let expired = matches!(
            checked,
            Err(Error::InvalidCertificate(
                CertificateError::Expired | CertificateError::ExpiredContext { .. }
            ))
        );
        let problem = match &checked {
            Ok(_) => None,
            Err(_) if expired => None,
            Err(e) => Some(e.to_string()),
        };
        if let Ok(mut seen) = self.seen.lock() {
            *seen = Some(PeerCertificate {
                der: end_entity.clone().into_owned(),
                problem: problem.clone(),
                expired,
            });
        }
        let untrusted = problem.is_some() || (expired && !self.accept_expired);
        if untrusted && self.enforce {
            checked
        } else {
            Ok(ServerCertVerified::assertion())
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

/// TLS handshake on `tcp` with `server_name` as SNI and for the name check. With `verify`, an
/// untrusted certificate fails the handshake; `accept_expired` lets an expired but otherwise trusted
/// one pass (the certificate sensor reports the days instead).
pub async fn connect_tls(
    tcp: TcpStream,
    server_name: &str,
    verify: bool,
    accept_expired: bool,
) -> Result<(TlsStream<TcpStream>, PeerCertificate), HttpError> {
    let name = ServerName::try_from(server_name.to_owned()).map_err(|_| HttpError::InvalidUrl)?;
    let recording = Arc::new(Recording {
        inner: web_pki_verifier()?,
        enforce: verify,
        accept_expired,
        seen: Mutex::new(None),
    });
    let mut config = ClientConfig::builder_with_provider(Arc::new(ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| HttpError::Tls(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(recording.clone())
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let stream = TlsConnector::from(Arc::new(config))
        .connect(name, tcp)
        .await
        .map_err(|e| HttpError::Tls(e.to_string()))?;
    let seen = recording
        .seen
        .lock()
        .ok()
        .and_then(|mut s| s.take())
        .ok_or_else(|| HttpError::Tls("no certificate".to_owned()))?;
    Ok((stream, seen))
}

#[cfg(test)]
pub(crate) mod testserver {
    //! Local HTTP(S) servers for tests: each connection gets the next canned response.

    use std::net::SocketAddr;
    use std::sync::Arc;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::mpsc;
    use tokio_rustls::TlsAcceptor;
    use tokio_rustls::rustls::ServerConfig;
    use tokio_rustls::rustls::crypto::ring;
    use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

    /// A self-signed certificate for `localhost` and `127.0.0.1`, valid `days` from now (negative = expired).
    pub fn certificate(days: i64) -> (CertificateDer<'static>, PrivateKeyDer<'static>) {
        let mut params =
            rcgen::CertificateParams::new(vec!["localhost".to_owned(), "127.0.0.1".to_owned()])
                .unwrap();
        let now = time::OffsetDateTime::now_utc();
        params.not_before = now - time::Duration::days(30);
        params.not_after = now + time::Duration::days(days);
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "opennetmon test CA");
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = params.self_signed(&key).unwrap();
        (
            cert.der().clone(),
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
        )
    }

    /// Serves `responses` (raw HTTP responses), one per connection, then stops accepting.
    /// With `tls`, it uses a certificate from [`certificate`] valid for `tls` days.
    pub async fn serve(responses: Vec<String>, tls: Option<i64>) -> SocketAddr {
        serve_and_record(responses, tls).await.0
    }

    /// Like [`serve`]; every request arrives on the receiver as text, head and body.
    pub async fn serve_and_record(
        responses: Vec<String>,
        tls: Option<i64>,
    ) -> (SocketAddr, mpsc::UnboundedReceiver<String>) {
        let (requests, received) = mpsc::unbounded_channel();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let acceptor = tls.map(|days| {
            let (cert, key) = certificate(days);
            let config = ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
                .with_safe_default_protocol_versions()
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(vec![cert], key)
                .unwrap();
            TlsAcceptor::from(Arc::new(config))
        });
        tokio::spawn(async move {
            for response in responses {
                let Ok((tcp, _)) = listener.accept().await else {
                    return;
                };
                match &acceptor {
                    Some(acceptor) => {
                        if let Ok(stream) = acceptor.accept(tcp).await {
                            answer(stream, &response, &requests).await;
                        }
                    }
                    None => answer(tcp, &response, &requests).await,
                }
            }
        });
        (addr, received)
    }

    /// Bytes of the body that follow the head of a request.
    fn content_length(head: &str) -> usize {
        head.lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, value)| value.trim().parse().ok())
            .unwrap_or(0)
    }

    async fn answer<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
        mut stream: S,
        response: &str,
        requests: &mpsc::UnboundedSender<String>,
    ) {
        let mut buf = vec![0u8; 4096];
        let mut seen = Vec::new();
        let head_end = loop {
            if let Some(at) = seen.windows(4).position(|w| w == b"\r\n\r\n") {
                break at + 4;
            }
            match stream.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(n) => seen.extend_from_slice(&buf[..n]),
            }
        };
        let total = head_end + content_length(&String::from_utf8_lossy(&seen[..head_end]));
        while seen.len() < total {
            match stream.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(n) => seen.extend_from_slice(&buf[..n]),
            }
        }
        let _ = requests.send(String::from_utf8_lossy(&seen).into_owned());
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.shutdown().await;
    }

    pub fn ok(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    pub fn status(code: u16, extra_headers: &str) -> String {
        format!(
            "HTTP/1.1 {code} X\r\n{extra_headers}Content-Length: 0\r\nConnection: close\r\n\r\n"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::testserver::{ok, serve, serve_and_record, status};
    use super::*;

    fn options() -> Options {
        Options {
            get: true,
            follow_redirects: true,
            verify_tls: true,
            max_body: 16,
            timeout: Duration::from_secs(5),
        }
    }

    #[test]
    fn urls_are_parsed_and_checked() {
        let t = parse_url("https://example.org").unwrap();
        assert_eq!(
            (t.tls, t.host.as_str(), t.port, t.path.as_str()),
            (true, "example.org", 443, "/")
        );
        let t = parse_url("http://[::1]:8080/a?b=1").unwrap();
        assert_eq!(
            (
                t.tls,
                t.host.as_str(),
                t.port,
                t.path.as_str(),
                t.authority.as_str()
            ),
            (false, "::1", 8080, "/a?b=1", "[::1]:8080")
        );
        for bad in [
            "",
            "example.org",
            "ftp://example.org/",
            "https://user:pass@example.org/",
            "https:///path",
            "https://exa mple.org/",
            "https://example.org/\n",
        ] {
            assert_eq!(parse_url(bad), Err(HttpError::InvalidUrl), "{bad:?}");
        }
        assert!(parse_url(&format!("https://e.org/{}", "a".repeat(MAX_URL_CHARS))).is_err());
    }

    #[test]
    fn redirect_locations_resolve_against_the_current_url() {
        let from = parse_url("https://example.org:8443/dir/page?x=1").unwrap();
        let to = |l: &str| {
            let t = redirect_target(&from, l).unwrap();
            format!("{}|{}|{}|{}", t.tls, t.host, t.port, t.path)
        };
        assert_eq!(to("/login"), "true|example.org|8443|/login");
        assert_eq!(to("other"), "true|example.org|8443|/dir/other");
        assert_eq!(to("//cdn.example.org/x"), "true|cdn.example.org|443|/x");
        assert_eq!(to("http://example.net/"), "false|example.net|80|/");
    }

    #[tokio::test]
    async fn api_calls_send_method_headers_and_body_and_stop_at_redirects() {
        let (addr, mut requests) =
            serve_and_record(vec![status(302, "Location: /next\r\n")], None).await;
        let url = format!("http://{addr}/control");
        let reply = call(&ApiCall {
            method: Method::POST,
            url: &url,
            headers: &[("soapaction", "urn:x#Get"), ("content-type", "text/xml")],
            body: "<a/>",
            max_body: 16,
            timeout: Duration::from_secs(5),
        })
        .await
        .unwrap();
        assert_eq!(reply.status, StatusCode::FOUND);
        let request = requests.recv().await.unwrap();
        assert!(
            request.starts_with("POST /control HTTP/1.1\r\n"),
            "{request}"
        );
        let lower = request.to_ascii_lowercase();
        assert!(lower.contains("soapaction: urn:x#get\r\n"), "{request}");
        assert!(lower.contains("content-length: 4\r\n"), "{request}");
        assert!(request.ends_with("\r\n\r\n<a/>"), "{request}");
    }

    #[tokio::test]
    async fn gets_status_and_the_start_of_the_body() {
        let addr = serve(vec![ok("hello world, a longer body")], None).await;
        let r = fetch(&format!("http://{addr}/"), &options()).await.unwrap();
        assert_eq!(r.status, StatusCode::OK);
        assert_eq!(r.body, b"hello world, a l");
    }

    #[tokio::test]
    async fn follows_redirects_only_when_asked_and_not_forever() {
        let addr = serve(vec![status(302, "Location: /next\r\n"), ok("there")], None).await;
        let r = fetch(&format!("http://{addr}/"), &options()).await.unwrap();
        assert_eq!(
            (r.status, r.body.as_slice()),
            (StatusCode::OK, &b"there"[..])
        );

        let addr = serve(vec![status(301, "Location: /x\r\n")], None).await;
        let no_follow = Options {
            follow_redirects: false,
            ..options()
        };
        let r = fetch(&format!("http://{addr}/"), &no_follow).await.unwrap();
        assert_eq!(r.status, StatusCode::MOVED_PERMANENTLY);

        let addr = serve(vec![status(302, "Location: /again\r\n"); 7], None).await;
        let r = fetch(&format!("http://{addr}/"), &options()).await;
        assert_eq!(r.unwrap_err(), HttpError::TooManyRedirects);
    }

    #[tokio::test]
    async fn self_signed_certificates_need_verification_switched_off() {
        let addr = serve(vec![ok("a"), ok("b")], Some(90)).await;
        let url = format!("https://localhost:{}/", addr.port());
        let refused = fetch(&url, &options()).await.unwrap_err();
        assert!(matches!(refused, HttpError::Tls(_)), "{refused:?}");
        let lax = Options {
            verify_tls: false,
            ..options()
        };
        let r = fetch(&url, &lax).await.unwrap();
        assert_eq!(r.body, b"b");
    }

    #[tokio::test]
    async fn the_handshake_reports_the_certificate_and_its_problem() {
        let addr = serve(vec![String::new()], Some(90)).await;
        let tcp = connect_tcp("127.0.0.1", addr.port()).await.unwrap();
        let (_, peer) = connect_tls(tcp, "localhost", false, false).await.unwrap();
        assert!(peer.problem.is_some());
        assert!(!peer.expired);
        assert!(!peer.der.is_empty());
    }

    #[tokio::test]
    async fn a_silent_server_runs_into_the_deadline() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let _keep = tokio::spawn(async move {
            let _conn = listener.accept().await;
            tokio::time::sleep(Duration::from_secs(30)).await;
        });
        let quick = Options {
            timeout: Duration::from_millis(200),
            ..options()
        };
        let r = fetch(&format!("http://{addr}/"), &quick).await;
        assert_eq!(r.unwrap_err(), HttpError::Timeout);
    }

    #[tokio::test]
    async fn closed_ports_fail_to_connect() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let r = fetch(&format!("http://127.0.0.1:{port}/"), &options()).await;
        assert!(matches!(r, Err(HttpError::Connect(_))), "{r:?}");
    }
}
