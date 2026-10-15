//! Static assets (CSS, JS) embedded into the binary; no CDN, no external files.

use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "static/"]
struct Assets;

pub async fn serve(Path(path): Path<String>) -> Response {
    match Assets::get(&path) {
        Some(file) => (
            [
                (header::CONTENT_TYPE, file.metadata.mimetype().to_owned()),
                (header::CACHE_CONTROL, "public, max-age=3600".to_owned()),
            ],
            file.data.into_owned(),
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
