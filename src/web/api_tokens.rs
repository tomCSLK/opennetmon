//! The page for the access tokens of the API (admins only): list, create, revoke. A new token is
//! shown once, in the reply to the form; only its hash is kept (`storage::api_tokens`).

use askama::Template;
use axum::Form;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::audit::{Action, Changes, Kind};
use crate::auth;
use crate::error::{Error, Result};
use crate::i18n::Lang;
use crate::storage::api_tokens as store;
use crate::web::api::new_token;
use crate::web::audit::{self, Object};
use crate::web::auth::{CsrfForm, CurrentUser};
use crate::web::{AppState, Layout, time_html};

struct TokenRow {
    id: i64,
    name: String,
    created_by: String,
    /// Time as `<time>` markup, shown in the browser's zone.
    created: String,
    /// Empty if the token was never used.
    last_used: String,
}

/// A token that was just created, shown once.
struct NewToken {
    name: String,
    token: String,
}

#[derive(Template)]
#[template(path = "api_tokens.html")]
struct PageTemplate {
    t: Lang,
    layout: Layout,
    tokens: Vec<TokenRow>,
    created: Option<NewToken>,
    flash: Option<String>,
    error: Option<String>,
    /// The name typed into the form, kept after an error.
    name: String,
}

#[derive(Deserialize)]
pub struct ListQuery {
    msg: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateForm {
    csrf_token: String,
    name: String,
}

async fn page(
    state: &AppState,
    user: &CurrentUser,
    created: Option<NewToken>,
    flash: Option<String>,
    error: Option<String>,
    name: String,
) -> Result<String> {
    let tokens = store::list(&state.pool)
        .await?
        .into_iter()
        .map(|t| TokenRow {
            id: t.id,
            name: t.name,
            created_by: t.created_by,
            created: time_html(t.created_ms),
            last_used: t.last_used_ms.map(time_html).unwrap_or_default(),
        })
        .collect();
    Ok(PageTemplate {
        t: user.lang,
        layout: user.layout(),
        tokens,
        created,
        flash,
        error,
        name,
    }
    .render()?)
}

pub async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(query): Query<ListQuery>,
) -> Result<Html<String>> {
    user.require_admin()?;
    let flash =
        (query.msg.as_deref() == Some("revoked")).then(|| user.lang.get("flash-api-token-revoked"));
    Ok(Html(
        page(&state, &user, None, flash, None, String::new()).await?,
    ))
}

pub async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Form(form): Form<CreateForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    let rejected = |message: crate::i18n::Msg| {
        let text = user.lang.msg(&message);
        (text, form.name.clone())
    };
    let name = match store::validate_name(&form.name) {
        Ok(name) => name,
        Err(message) => {
            let (text, name) = rejected(message);
            let body = page(&state, &user, None, None, Some(text), name).await?;
            return Ok((StatusCode::UNPROCESSABLE_ENTITY, Html(body)).into_response());
        }
    };
    let token = new_token();
    let now_ms = auth::now_secs().saturating_mul(1000);
    let id = match store::create(
        &state.pool,
        &name,
        &auth::token_hash(&token),
        &user.username,
        now_ms,
    )
    .await
    {
        Ok(id) => id,
        Err(Error::Conflict(message)) => {
            let (text, name) = rejected(message);
            let body = page(&state, &user, None, None, Some(text), name).await?;
            return Ok((StatusCode::UNPROCESSABLE_ENTITY, Html(body)).into_response());
        }
        Err(e) => return Err(e),
    };
    tracing::info!(user = %user.username, token = id, "API token created");
    // The token itself never goes in: the entry only names the token.
    audit::record(
        &state,
        &user,
        Action::Created,
        Object::new(Kind::ApiToken, id, &name),
        Changes::new(),
    )
    .await;
    let created = NewToken { name, token };
    let body = page(&state, &user, Some(created), None, None, String::new()).await?;
    Ok(Html(body).into_response())
}

pub async fn delete(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<CsrfForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    let token = store::get(&state.pool, id).await?.ok_or(Error::NotFound)?;
    if !store::delete(&state.pool, id).await? {
        return Err(Error::NotFound);
    }
    tracing::info!(user = %user.username, token = id, "API token revoked");
    audit::record(
        &state,
        &user,
        Action::Deleted,
        Object::new(Kind::ApiToken, id, &token.name),
        Changes::new(),
    )
    .await;
    Ok(Redirect::to("/api-tokens?msg=revoked").into_response())
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};

    use crate::web::testutil::*;

    fn bearer(token: &str) -> Request<Body> {
        Request::builder()
            .uri("/api/v1/inventory")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    }

    /// The token shown on the page of a new token.
    fn shown_token(body: &str) -> String {
        let start = body.find("id=\"new-token\" class=\"token\">").unwrap() + 29;
        body[start..].split('<').next().unwrap().to_owned()
    }

    #[tokio::test]
    async fn an_admin_creates_a_token_that_is_shown_once_and_revokes_it() {
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;

        let empty = send(&h.app, get("/api-tokens", Some(&admin))).await;
        assert_eq!(empty.status, StatusCode::OK);
        assert!(empty.body.contains("Noch kein Token"), "{}", empty.body);

        let created = send(
            &h.app,
            post(
                "/api-tokens",
                Some(&admin),
                &format!("csrf_token={csrf}&name=opensecmon"),
            ),
        )
        .await;
        assert_eq!(created.status, StatusCode::OK);
        let token = shown_token(&created.body);
        assert!(token.starts_with("onm_") && token.len() == 68, "{token}");
        assert!(created.body.contains("Neuer Token „opensecmon“"));
        assert!(
            send(&h.app, bearer(&token)).await.status == StatusCode::OK,
            "the new token works"
        );

        // Afterwards the page lists the name, never the token; the database has only a hash.
        let list = send(&h.app, get("/api-tokens", Some(&admin))).await;
        assert!(list.body.contains("opensecmon") && !list.body.contains(&token));
        let stored: String = sqlx::query_scalar("SELECT token_hash FROM api_tokens")
            .fetch_one(&h.state.pool)
            .await
            .unwrap();
        assert_ne!(stored, token);
        assert_eq!(stored, crate::auth::token_hash(&token));

        let revoked = send(
            &h.app,
            post(
                "/api-tokens/1/delete",
                Some(&admin),
                &format!("csrf_token={csrf}"),
            ),
        )
        .await;
        assert_eq!(revoked.status, StatusCode::SEE_OTHER);
        assert_eq!(revoked.location(), "/api-tokens?msg=revoked");
        assert_eq!(
            send(&h.app, bearer(&token)).await.status,
            StatusCode::UNAUTHORIZED,
            "a revoked token stops working"
        );
        let after = send(&h.app, get("/api-tokens?msg=revoked", Some(&admin))).await;
        assert!(after.body.contains("Token widerrufen."));

        // Both changes are in the audit log, without the token.
        let entries: Vec<(String, String, String)> =
            sqlx::query_as("SELECT action, object_name, detail FROM audit_log WHERE kind = 'api_token' ORDER BY id")
                .fetch_all(&h.state.pool)
                .await
                .unwrap();
        let actions: Vec<_> = entries
            .iter()
            .map(|(a, n, _)| (a.as_str(), n.as_str()))
            .collect();
        assert_eq!(
            actions,
            [("created", "opensecmon"), ("deleted", "opensecmon")]
        );
        assert!(
            entries
                .iter()
                .all(|(_, _, detail)| !detail.contains(&token))
        );
    }

    #[tokio::test]
    async fn names_are_checked_and_unique() {
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;
        let make = |name: &str| {
            post(
                "/api-tokens",
                Some(&admin),
                &format!("csrf_token={csrf}&name={name}"),
            )
        };
        assert_eq!(send(&h.app, make("one")).await.status, StatusCode::OK);
        let twice = send(&h.app, make("ONE")).await;
        assert_eq!(twice.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            twice
                .body
                .contains("Es gibt schon einen Token mit diesem Namen.")
        );
        assert!(
            !twice.body.contains("class=\"token\""),
            "no token for a rejected name"
        );
        let blank = send(&h.app, make("%20%20")).await;
        assert_eq!(blank.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(blank.body.contains("1 bis 64 Zeichen"));
        let long = send(&h.app, make(&"x".repeat(65))).await;
        assert_eq!(long.status, StatusCode::UNPROCESSABLE_ENTITY);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM api_tokens")
            .fetch_one(&h.state.pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn only_admins_with_a_valid_form_manage_tokens() {
        let h = harness(true).await;
        let (viewer, viewer_csrf) = login(&h, "bert").await;
        assert_eq!(
            send(&h.app, get("/api-tokens", Some(&viewer))).await.status,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            send(
                &h.app,
                post(
                    "/api-tokens",
                    Some(&viewer),
                    &format!("csrf_token={viewer_csrf}&name=x")
                )
            )
            .await
            .status,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            send(&h.app, get("/api-tokens", None)).await.status,
            StatusCode::SEE_OTHER
        );
        let (admin, admin_csrf) = login(&h, "anna").await;
        let bad = send(
            &h.app,
            post("/api-tokens", Some(&admin), "csrf_token=wrong&name=x"),
        )
        .await;
        assert_eq!(bad.status, StatusCode::FORBIDDEN);
        let missing = send(
            &h.app,
            post(
                "/api-tokens/99/delete",
                Some(&admin),
                &format!("csrf_token={admin_csrf}"),
            ),
        )
        .await;
        assert_eq!(missing.status, StatusCode::NOT_FOUND);
    }
}
