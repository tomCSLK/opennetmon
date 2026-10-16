//! Static assets (CSS, JS) embedded into the binary; no CDN, no external files.
//!
//! Every response carries an ETag (content hash) and `Cache-Control: no-cache`: the browser keeps its
//! copy but asks on each page load whether it is still current, and gets a tiny `304 Not Modified`
//! if so. After an update the new files are therefore used immediately, without a forced reload.

use axum::extract::Path;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "static/"]
struct Assets;

const CACHE_CONTROL: &str = "no-cache";

fn etag(hash: &[u8; 32]) -> String {
    // 16 bytes of the SHA-256 are plenty to tell versions apart.
    let hex: String = hash[..16].iter().map(|b| format!("{b:02x}")).collect();
    format!("\"{hex}\"")
}

/// Whether `If-None-Match` names the current version (also handles lists and `*`).
fn not_modified(headers: &HeaderMap, current: &str) -> bool {
    headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|tag| tag.trim().trim_start_matches("W/"))
        .any(|tag| tag == current || tag == "*")
}

pub async fn serve(Path(path): Path<String>, headers: HeaderMap) -> Response {
    let Some(file) = Assets::get(&path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let tag = etag(&file.metadata.sha256_hash());
    if not_modified(&headers, &tag) {
        return (
            StatusCode::NOT_MODIFIED,
            [
                (header::ETAG, tag),
                (header::CACHE_CONTROL, CACHE_CONTROL.to_owned()),
            ],
        )
            .into_response();
    }
    (
        [
            (header::CONTENT_TYPE, file.metadata.mimetype().to_owned()),
            (header::ETAG, tag),
            (header::CACHE_CONTROL, CACHE_CONTROL.to_owned()),
        ],
        file.data.into_owned(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::testutil::*;

    #[tokio::test]
    async fn assets_are_revalidated_with_an_etag() {
        let h = harness(true).await;
        let first = send(&h.app, get("/static/app.css", None)).await;
        assert_eq!(first.status, StatusCode::OK);
        assert_eq!(first.headers[header::CACHE_CONTROL], "no-cache");
        let tag = first.headers[header::ETAG].to_str().unwrap().to_owned();
        assert!(tag.starts_with('"') && tag.len() == 34, "{tag}");

        let revalidate = |value: &str| {
            axum::http::Request::builder()
                .uri("/static/app.css")
                .header(header::IF_NONE_MATCH, value)
                .body(axum::body::Body::empty())
                .unwrap()
        };
        let unchanged = send(&h.app, revalidate(&tag)).await;
        assert_eq!(unchanged.status, StatusCode::NOT_MODIFIED);
        assert!(unchanged.body.is_empty());
        let weak_in_list = send(&h.app, revalidate(&format!("\"old\", W/{tag}"))).await;
        assert_eq!(weak_in_list.status, StatusCode::NOT_MODIFIED);

        let changed = send(&h.app, revalidate("\"0000\"")).await;
        assert_eq!(changed.status, StatusCode::OK);
        assert!(!changed.body.is_empty());
    }

    #[tokio::test]
    async fn different_files_have_different_etags_and_unknown_files_are_404() {
        let h = harness(true).await;
        let a = send(&h.app, get("/static/app.css", None)).await;
        let b = send(&h.app, get("/static/chart.js", None)).await;
        assert_ne!(a.headers[header::ETAG], b.headers[header::ETAG]);
        assert_eq!(
            send(&h.app, get("/static/nope.js", None)).await.status,
            StatusCode::NOT_FOUND
        );
    }
}
