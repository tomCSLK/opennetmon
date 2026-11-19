//! `opennetmon healthcheck`: asks the running server for `/healthz` on its own address, for the health
//! check of the container image (it has no shell and no curl). An address that listens on every interface
//! is reached over loopback. With built-in HTTPS the certificate is not checked: the other end is this
//! server, and the request carries nothing secret.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use crate::config::Config;
use crate::http_client::{self, Options};

/// Time the server gets to answer.
const TIMEOUT: Duration = Duration::from_secs(5);

/// The address of `/healthz` for a server listening on `listen`.
pub fn url(listen: SocketAddr, tls: bool) -> String {
    let ip = match listen.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    let scheme = if tls { "https" } else { "http" };
    format!("{scheme}://{}/healthz", SocketAddr::new(ip, listen.port()))
}

/// `Ok` when the server answers `/healthz` with 200, else what went wrong.
pub async fn check(config: &Config) -> Result<(), String> {
    check_url(&url(config.listen, config.tls_files().is_some())).await
}

async fn check_url(url: &str) -> Result<(), String> {
    let options = Options {
        get: true,
        follow_redirects: false,
        verify_tls: false,
        max_body: 64,
        timeout: TIMEOUT,
    };
    let response = http_client::fetch(url, &options)
        .await
        .map_err(|e| format!("{url}: {e}"))?;
    if response.status.is_success() {
        Ok(())
    } else {
        Err(format!("{url}: status {}", response.status.as_u16()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http_client::testserver;

    #[test]
    fn any_address_is_reached_over_loopback() {
        let cases = [
            ("0.0.0.0:8443", true, "https://127.0.0.1:8443/healthz"),
            ("[::]:8443", true, "https://[::1]:8443/healthz"),
            ("127.0.0.1:8080", false, "http://127.0.0.1:8080/healthz"),
            ("192.0.2.5:9000", false, "http://192.0.2.5:9000/healthz"),
        ];
        for (listen, tls, expected) in cases {
            assert_eq!(url(listen.parse().unwrap(), tls), expected);
        }
    }

    #[tokio::test]
    async fn a_self_signed_server_that_answers_200_is_healthy() {
        let addr = testserver::serve(vec![testserver::ok("ok")], Some(30)).await;
        assert_eq!(check_url(&url(addr, true)).await, Ok(()));
    }

    #[tokio::test]
    async fn an_error_status_or_no_server_is_unhealthy() {
        let addr = testserver::serve(vec![testserver::status(503, "")], None).await;
        let err = check_url(&url(addr, false)).await.unwrap_err();
        assert!(err.ends_with("status 503"), "{err}");

        let closed = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        assert!(check_url(&url(closed, false)).await.is_err());
    }
}
