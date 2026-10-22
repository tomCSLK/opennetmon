//! Group pages (admin only): create, rename/move, delete.

use askama::Template;
use axum::Form;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::devices::subtree_ids;
use crate::error::{Error, Result};
use crate::i18n::{Lang, Msg};
use crate::storage::groups as store;
use crate::web::auth::CurrentUser;
use crate::web::{AppState, Layout, SelectOption, back_to_group, group_options};

#[derive(Template)]
#[template(path = "group_form.html")]
struct FormTemplate {
    /// Language of the page.
    t: Lang,
    layout: Layout,
    /// Catalog id of the heading.
    heading: &'static str,
    action: String,
    delete_action: Option<String>,
    error: Option<String>,
    name: String,
    parents: Vec<SelectOption>,
    /// `back` of the form: "overview" or the group page (`group-<id>`) it was opened from.
    back: String,
    cancel_href: String,
}

#[derive(Deserialize)]
pub struct GroupForm {
    csrf_token: String,
    name: String,
    #[serde(default)]
    parent_id: String,
    #[serde(default)]
    back: String,
}

/// `?parent=<id>` preselects the parent of a new group; `back` is the page to return to.
#[derive(Deserialize)]
pub struct FormQuery {
    parent: Option<i64>,
    #[serde(default)]
    back: String,
}

/// The group page to return to, or the overview.
fn back_href(back: &str) -> String {
    back_to_group(back).map_or_else(|| "/".to_owned(), |id| format!("/groups/{id}"))
}

/// The value of the hidden `back` field (unknown values become "overview").
fn back_value(back: &str) -> String {
    back_to_group(back).map_or_else(|| "overview".to_owned(), |id| format!("group-{id}"))
}

#[derive(Deserialize)]
pub struct DeleteForm {
    csrf_token: String,
    confirm: Option<String>,
}

fn parse_optional_id(value: &str) -> Result<Option<i64>> {
    match value.trim() {
        "" => Ok(None),
        text => text
            .parse()
            .map(Some)
            .map_err(|_| Error::invalid(Msg::new("group-invalid"))),
    }
}

pub async fn new_form(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(query): Query<FormQuery>,
) -> Result<Html<String>> {
    user.require_admin()?;
    let groups = store::list_groups(&state.pool).await?;
    let page = FormTemplate {
        layout: user.layout(),
        t: user.lang,
        heading: "group-new",
        action: "/groups".to_owned(),
        delete_action: None,
        error: None,
        name: String::new(),
        parents: group_options(&groups, query.parent, &[]),
        back: back_value(&query.back),
        cancel_href: back_href(&query.back),
    };
    Ok(Html(page.render()?))
}

pub async fn edit_form(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Query(query): Query<FormQuery>,
) -> Result<Html<String>> {
    user.require_admin()?;
    let groups = store::list_groups(&state.pool).await?;
    let group = groups.iter().find(|g| g.id == id).ok_or(Error::NotFound)?;
    let page = FormTemplate {
        layout: user.layout(),
        t: user.lang,
        heading: "group-edit",
        action: format!("/groups/{id}"),
        delete_action: Some(format!("/groups/{id}/delete")),
        error: None,
        name: group.name.clone(),
        parents: group_options(&groups, group.parent_id, &subtree_ids(&groups, id)),
        back: back_value(&query.back),
        cancel_href: back_href(&query.back),
    };
    Ok(Html(page.render()?))
}

pub async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Form(form): Form<GroupForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    save(&state, &user, None, &form).await
}

pub async fn update(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<GroupForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    save(&state, &user, Some(id), &form).await
}

async fn save(
    state: &AppState,
    user: &CurrentUser,
    id: Option<i64>,
    form: &GroupForm,
) -> Result<Response> {
    let outcome = match parse_optional_id(&form.parent_id) {
        Ok(parent) => match id {
            Some(id) => store::update_group(&state.pool, id, &form.name, parent)
                .await
                .map(|()| id),
            None => store::create_group(&state.pool, &form.name, parent).await,
        },
        Err(e) => Err(e),
    };
    match outcome {
        Ok(saved) => {
            tracing::info!(user = %user.username, group = saved, "group saved");
            let msg = if id.is_some() {
                "group_updated"
            } else {
                "group_created"
            };
            let page = match back_to_group(&form.back) {
                Some(group) => format!("/groups/{group}"),
                None => "/".to_owned(),
            };
            Ok(Redirect::to(&format!("{page}?msg={msg}")).into_response())
        }
        Err(Error::Invalid(message) | Error::Conflict(message)) => {
            let groups = store::list_groups(&state.pool).await?;
            let exclude = id.map(|id| subtree_ids(&groups, id)).unwrap_or_default();
            let page = FormTemplate {
                layout: user.layout(),
                t: user.lang,
                heading: if id.is_some() {
                    "group-edit"
                } else {
                    "group-new"
                },
                action: id.map_or_else(|| "/groups".to_owned(), |id| format!("/groups/{id}")),
                delete_action: id.map(|id| format!("/groups/{id}/delete")),
                error: Some(user.lang.msg(&message)),
                name: form.name.clone(),
                parents: group_options(
                    &groups,
                    parse_optional_id(&form.parent_id).ok().flatten(),
                    &exclude,
                ),
                back: back_value(&form.back),
                cancel_href: back_href(&form.back),
            };
            Ok((StatusCode::UNPROCESSABLE_ENTITY, Html(page.render()?)).into_response())
        }
        Err(e) => Err(e),
    }
}

pub async fn delete(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<DeleteForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    if form.confirm.as_deref() != Some("yes") {
        return Err(Error::invalid(Msg::new("confirm-delete")));
    }
    store::delete_group(&state.pool, id).await?;
    tracing::info!(user = %user.username, group = id, "group deleted");
    Ok(Redirect::to("/?msg=group_deleted").into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::testutil::*;

    fn form(csrf: &str, name: &str, parent: &str) -> String {
        format!("csrf_token={csrf}&name={name}&parent_id={parent}")
    }

    #[tokio::test]
    async fn only_admins_may_manage_groups() {
        let h = harness(true).await;
        for path in ["/groups/new", "/groups/1/edit"] {
            assert_eq!(
                send(&h.app, get(path, None)).await.status,
                StatusCode::SEE_OTHER,
                "{path}"
            );
        }
        let (viewer, csrf) = login(&h, "bert").await;
        for path in ["/groups/new", "/groups/1/edit"] {
            assert_eq!(
                send(&h.app, get(path, Some(&viewer))).await.status,
                StatusCode::FORBIDDEN,
                "{path}"
            );
        }
        let reply = send(
            &h.app,
            post("/groups", Some(&viewer), &form(&csrf, "X", "")),
        )
        .await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN);
        assert!(store::list_groups(&h.state.pool).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn groups_are_created_nested_and_shown_in_tree_order() {
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;
        let reply = send(
            &h.app,
            post("/groups", Some(&admin), &form(&csrf, "Zuhause", "")),
        )
        .await;
        assert_eq!(reply.location(), "/?msg=group_created");
        send(
            &h.app,
            post("/groups", Some(&admin), &form(&csrf, "Keller", "1")),
        )
        .await;
        send(
            &h.app,
            post("/groups", Some(&admin), &form(&csrf, "Anderes", "")),
        )
        .await;

        let home = send(&h.app, get("/?msg=group_created", Some(&admin))).await;
        assert!(home.body.contains("Gruppe angelegt."));
        let pos = |needle: &str| {
            home.body
                .find(needle)
                .unwrap_or_else(|| panic!("{needle} missing"))
        };
        assert!(pos("Anderes") < pos("Zuhause") && pos("Zuhause") < pos("Keller"));
        // Keller is nested inside the Zuhause node.
        let zuhause = pos("data-group=\"1\"");
        let keller = pos("<details class=\"node\" id=\"group-2\" data-group=\"2\"");
        assert!(zuhause < keller);
        assert!(home.body.contains("Keine Geräte in dieser Gruppe."));
        assert!(home.body.contains("/groups/1/edit"));

        let new_page = send(&h.app, get("/groups/new", Some(&admin))).await;
        assert!(new_page.body.contains("Zuhause") && new_page.body.contains("Keller"));
    }

    #[tokio::test]
    async fn duplicate_cycle_and_empty_names_show_messages() {
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;
        send(&h.app, post("/groups", Some(&admin), &form(&csrf, "A", ""))).await;
        send(
            &h.app,
            post("/groups", Some(&admin), &form(&csrf, "B", "1")),
        )
        .await;

        let dup = send(&h.app, post("/groups", Some(&admin), &form(&csrf, "a", ""))).await;
        assert_eq!(dup.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(dup.body.contains("bereits eine Gruppe"));

        let empty = send(&h.app, post("/groups", Some(&admin), &form(&csrf, "+", ""))).await;
        assert_eq!(empty.status, StatusCode::UNPROCESSABLE_ENTITY);

        // Move A below its own child B.
        let cycle = send(
            &h.app,
            post("/groups/1", Some(&admin), &form(&csrf, "A", "2")),
        )
        .await;
        assert_eq!(cycle.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(cycle.body.contains("nicht in sich selbst"));

        let bad_parent = send(
            &h.app,
            post("/groups", Some(&admin), &form(&csrf, "C", "abc")),
        )
        .await;
        assert_eq!(bad_parent.status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn edit_page_hides_the_subtree_as_parent_and_update_renames() {
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;
        send(
            &h.app,
            post("/groups", Some(&admin), &form(&csrf, "Haus", "")),
        )
        .await;
        send(
            &h.app,
            post("/groups", Some(&admin), &form(&csrf, "Etage", "1")),
        )
        .await;
        send(
            &h.app,
            post("/groups", Some(&admin), &form(&csrf, "Garten", "")),
        )
        .await;

        let edit = send(&h.app, get("/groups/1/edit", Some(&admin))).await;
        let options: Vec<&str> = edit
            .body
            .match_indices("<option")
            .map(|(i, _)| &edit.body[i..i + 60])
            .collect();
        assert!(
            !options
                .iter()
                .any(|o| o.contains("Etage") || o.contains(">Haus"))
        );
        assert!(edit.body.contains("Garten"));

        let reply = send(
            &h.app,
            post("/groups/1", Some(&admin), &form(&csrf, "Wohnhaus", "3")),
        )
        .await;
        assert_eq!(reply.location(), "/?msg=group_updated");
        let moved = store::get_group(&h.state.pool, 1).await.unwrap().unwrap();
        assert_eq!(
            (moved.name.as_str(), moved.parent_id),
            ("Wohnhaus", Some(3))
        );
        assert_eq!(
            send(&h.app, get("/groups/99/edit", Some(&admin)))
                .await
                .status,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn delete_needs_csrf_and_confirmation_and_an_empty_group() {
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;
        send(&h.app, post("/groups", Some(&admin), &form(&csrf, "P", ""))).await;
        send(
            &h.app,
            post("/groups", Some(&admin), &form(&csrf, "K", "1")),
        )
        .await;

        let bad = send(
            &h.app,
            post(
                "/groups/2/delete",
                Some(&admin),
                "csrf_token=falsch&confirm=yes",
            ),
        )
        .await;
        assert_eq!(bad.status, StatusCode::FORBIDDEN);
        let unconfirmed = send(
            &h.app,
            post(
                "/groups/2/delete",
                Some(&admin),
                &format!("csrf_token={csrf}"),
            ),
        )
        .await;
        assert_eq!(unconfirmed.status, StatusCode::UNPROCESSABLE_ENTITY);
        let not_empty = send(
            &h.app,
            post(
                "/groups/1/delete",
                Some(&admin),
                &format!("csrf_token={csrf}&confirm=yes"),
            ),
        )
        .await;
        assert_eq!(not_empty.status, StatusCode::CONFLICT);

        let ok = send(
            &h.app,
            post(
                "/groups/2/delete",
                Some(&admin),
                &format!("csrf_token={csrf}&confirm=yes"),
            ),
        )
        .await;
        assert_eq!(ok.location(), "/?msg=group_deleted");
        assert_eq!(store::list_groups(&h.state.pool).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn group_names_are_html_escaped() {
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;
        send(
            &h.app,
            post(
                "/groups",
                Some(&admin),
                &form(&csrf, "%3Cb%3Ex%3C%2Fb%3E", ""),
            ),
        )
        .await;
        let home = send(&h.app, get("/", Some(&admin))).await;
        assert!(!home.body.contains("<b>x</b>"));
    }
}
