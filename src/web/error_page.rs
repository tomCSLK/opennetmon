//! Error pages: turns failed page requests (plain-text errors of the handlers, axum's rejections,
//! unknown addresses) into a page in the design of the interface. Signed-in users get it with the
//! sidebar, everyone else in the card of the login page. Requests of the scripts (JSON) and pages
//! that already explain the error themselves (forms shown again) are left alone.

use askama::Template;
use axum::extract::{FromRequestParts, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};

use crate::error::ErrorMessage;
use crate::i18n::{self, Lang};
use crate::web::auth::CurrentUser;
use crate::web::{AppState, Layout};

/// What the page says, the same for both layouts.
struct ErrorView {
    status: u16,
    heading: String,
    message: String,
    /// Where "Back" leads without JavaScript: the previous page of this site, otherwise the overview.
    back: String,
}

#[derive(Template)]
#[template(path = "error_page.html")]
struct SignedInTemplate {
    t: Lang,
    layout: Layout,
    error: ErrorView,
}

#[derive(Template)]
#[template(path = "error_page_public.html")]
struct PublicTemplate {
    t: Lang,
    error: ErrorView,
}

/// Middleware: renders error responses of page requests as an error page.
pub async fn render(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let headers = request.headers().clone();
    let response = next.run(request).await;
    let status = response.status();
    if !(status.is_client_error() || status.is_server_error())
        || !wants_html(&headers)
        || has_page(&response)
    {
        return response;
    }
    let message = response.extensions().get::<ErrorMessage>().cloned();
    // The handler may already have switched to the user's language; the session decides the layout.
    let (mut parts, ()) = Request::new(()).into_parts();
    parts.headers = headers.clone();
    let user = CurrentUser::from_request_parts(&mut parts, &state)
        .await
        .ok();
    let t = user.as_ref().map_or_else(i18n::current, |u| u.lang);
    let error = ErrorView {
        status: status.as_u16(),
        heading: t.get(heading_id(status)),
        message: message.map_or_else(|| t.get(message_id(status)), |m| m.0),
        back: back_link(&headers),
    };
    let html = match user {
        Some(user) => SignedInTemplate {
            t,
            layout: user.layout(),
            error,
        }
        .render(),
        None => PublicTemplate { t, error }.render(),
    };
    match html {
        Ok(html) => {
            let mut page = (status, Html(html)).into_response();
            // Keep what the original response asked for, e.g. Allow on 405.
            for name in [header::ALLOW, header::RETRY_AFTER] {
                if let Some(value) = response.headers().get(&name) {
                    page.headers_mut().insert(name, value.clone());
                }
            }
            page
        }
        Err(e) => {
            tracing::error!(error = %e, "error page failed");
            response
        }
    }
}

/// Browsers ask for HTML when they open a page; `fetch` from the scripts asks for JSON.
fn wants_html(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("text/html"))
}

/// The response is a page or data already: a form shown again with its error, or JSON.
fn has_page(response: &Response) -> bool {
    response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/html") || v.starts_with("application/json"))
}

fn heading_id(status: StatusCode) -> &'static str {
    match status {
        StatusCode::NOT_FOUND => "error-heading-not-found",
        StatusCode::FORBIDDEN => "error-heading-forbidden",
        StatusCode::BAD_REQUEST
        | StatusCode::UNPROCESSABLE_ENTITY
        | StatusCode::CONFLICT
        | StatusCode::PAYLOAD_TOO_LARGE
        | StatusCode::METHOD_NOT_ALLOWED => "error-heading-input",
        s if s.is_server_error() => "error-heading-internal",
        _ => "error-heading",
    }
}

/// The message when the response does not bring its own (axum's texts are English and technical).
fn message_id(status: StatusCode) -> &'static str {
    match status {
        StatusCode::NOT_FOUND => "error-not-found-text",
        StatusCode::FORBIDDEN => "error-forbidden",
        StatusCode::PAYLOAD_TOO_LARGE => "form-too-large",
        StatusCode::METHOD_NOT_ALLOWED => "error-method",
        s if s.is_server_error() => "error-internal-text",
        _ => "error-bad-request",
    }
}

/// The page the request came from, if it is on this site (path and query only).
fn back_link(headers: &HeaderMap) -> String {
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    headers
        .get(header::REFERER)
        .and_then(|v| v.to_str().ok())
        .and_then(|referer| {
            let rest = referer
                .strip_prefix("https://")
                .or_else(|| referer.strip_prefix("http://"))?;
            let (authority, path) = rest.split_at(rest.find('/')?);
            (Some(authority) == host && !path.starts_with("//")).then(|| path.to_owned())
        })
        .unwrap_or_else(|| "/".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::testutil::{get, harness, login, post, send};
    use axum::http::HeaderValue;

    fn headers(pairs: &[(header::HeaderName, &'static str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (name, value) in pairs {
            h.insert(name.clone(), HeaderValue::from_static(value));
        }
        h
    }

    #[test]
    fn back_leads_to_the_previous_page_of_this_site_only() {
        let same = headers(&[
            (header::HOST, "monitor.example:8443"),
            (
                header::REFERER,
                "https://monitor.example:8443/devices/3/edit?x=1",
            ),
        ]);
        assert_eq!(back_link(&same), "/devices/3/edit?x=1");
        let other = headers(&[
            (header::HOST, "monitor.example:8443"),
            (header::REFERER, "https://evil.example/devices"),
        ]);
        assert_eq!(back_link(&other), "/");
        let tricky = headers(&[
            (header::HOST, "monitor.example"),
            (header::REFERER, "https://monitor.example//evil.example/x"),
        ]);
        assert_eq!(back_link(&tricky), "/");
        assert_eq!(
            back_link(&headers(&[(header::HOST, "monitor.example")])),
            "/"
        );
    }

    fn page(
        mut request: axum::http::Request<axum::body::Body>,
    ) -> axum::http::Request<axum::body::Body> {
        request.headers_mut().insert(
            header::ACCEPT,
            HeaderValue::from_static("text/html,application/xhtml+xml,*/*;q=0.8"),
        );
        request
    }

    #[tokio::test]
    async fn signed_in_users_get_the_page_with_the_sidebar() {
        let h = harness(true).await;
        let (viewer, _) = login(&h, "bert").await;
        let reply = send(&h.app, page(get("/sensors/99", Some(&viewer)))).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND);
        assert!(
            reply.headers[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("text/html")
        );
        for needle in [
            "class=\"sidebar\"",
            "Fehler 404",
            "<h1>Seite nicht gefunden</h1>",
            "Nicht gefunden",
            "data-back",
        ] {
            assert!(
                reply.body.contains(needle),
                "{needle} missing: {}",
                reply.body
            );
        }
        // The security headers apply to the new page as well.
        assert!(reply.headers.contains_key(header::CONTENT_SECURITY_POLICY));
    }

    #[tokio::test]
    async fn others_get_the_page_of_the_login_card_and_scripts_keep_plain_errors() {
        let h = harness(true).await;
        let reply = send(&h.app, page(get("/no/such/page", None))).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND);
        assert!(reply.body.contains("class=\"auth-card\""), "{}", reply.body);
        assert!(reply.body.contains("Diese Seite gibt es nicht (mehr)."));
        assert!(!reply.body.contains("class=\"sidebar\""));

        let (viewer, _) = login(&h, "bert").await;
        let mut json = get("/sensors/99/data", Some(&viewer));
        json.headers_mut()
            .insert(header::ACCEPT, HeaderValue::from_static("application/json"));
        let reply = send(&h.app, json).await;
        assert_eq!(reply.status, StatusCode::NOT_FOUND);
        assert_eq!(reply.body, "Nicht gefunden");
    }

    #[tokio::test]
    async fn unreadable_forms_get_a_readable_message_instead_of_axums_text() {
        let h = harness(true).await;
        let (admin, _) = login(&h, "anna").await;
        // The rename form without its fields: axum rejects it before the handler runs.
        let reply = send(&h.app, page(post("/sensors/1/rename", Some(&admin), "x=1"))).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            reply.body.contains("<h1>Eingabe nicht möglich</h1>"),
            "{}",
            reply.body
        );
        assert!(reply.body.contains("Bitte die Eingaben prüfen"));
        assert!(!reply.body.contains("deserialize"), "{}", reply.body);
    }

    #[tokio::test]
    async fn the_message_of_the_handler_is_shown() {
        use axum::routing::get as route_get;
        let h = harness(true).await;
        let app = axum::Router::new()
            .route(
                "/x",
                route_get(|| async { Err::<(), _>(crate::error::Error::invalid("zu kurz")) }),
            )
            .layer(axum::middleware::from_fn_with_state(
                h.state.clone(),
                render,
            ))
            .with_state(h.state.clone());
        let reply = send(&app, page(get("/x", None))).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            reply
                .body
                .contains("<p class=\"error\" role=\"alert\">zu kurz</p>"),
            "{}",
            reply.body
        );
    }

    #[test]
    fn only_page_requests_get_a_page() {
        assert!(wants_html(&headers(&[(
            header::ACCEPT,
            "text/html,application/xhtml+xml,*/*;q=0.8"
        )])));
        assert!(!wants_html(&headers(&[(
            header::ACCEPT,
            "application/json"
        )])));
        assert!(!wants_html(&HeaderMap::new()));
    }
}
