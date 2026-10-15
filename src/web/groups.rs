//! Group pages (admin only): create, rename/move, delete.

use askama::Template;
use axum::Form;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::devices::subtree_ids;
use crate::error::{Error, Result};
use crate::storage::groups as store;
use crate::web::auth::CurrentUser;
use crate::web::{AppState, Layout, SelectOption, group_options};

#[derive(Template)]
#[template(path = "group_form.html")]
struct FormTemplate {
    layout: Layout,
    heading: &'static str,
    action: String,
    delete_action: Option<String>,
    error: Option<String>,
    name: String,
    parents: Vec<SelectOption>,
}

#[derive(Deserialize)]
pub struct GroupForm {
    csrf_token: String,
    name: String,
    #[serde(default)]
    parent_id: String,
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
            .map_err(|_| Error::Invalid("Ungültige Gruppe.".to_owned())),
    }
}

pub async fn new_form(State(state): State<AppState>, user: CurrentUser) -> Result<Html<String>> {
    user.require_admin()?;
    let groups = store::list_groups(&state.pool).await?;
    let page = FormTemplate {
        layout: user.layout(),
        heading: "Neue Gruppe",
        action: "/groups".to_owned(),
        delete_action: None,
        error: None,
        name: String::new(),
        parents: group_options(&groups, None, &[]),
    };
    Ok(Html(page.render()?))
}

pub async fn edit_form(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i64>,
) -> Result<Html<String>> {
    user.require_admin()?;
    let groups = store::list_groups(&state.pool).await?;
    let group = groups.iter().find(|g| g.id == id).ok_or(Error::NotFound)?;
    let page = FormTemplate {
        layout: user.layout(),
        heading: "Gruppe bearbeiten",
        action: format!("/groups/{id}"),
        delete_action: Some(format!("/groups/{id}/delete")),
        error: None,
        name: group.name.clone(),
        parents: group_options(&groups, group.parent_id, &subtree_ids(&groups, id)),
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
            Ok(Redirect::to(&format!("/?msg={msg}")).into_response())
        }
        Err(Error::Invalid(message) | Error::Conflict(message)) => {
            let groups = store::list_groups(&state.pool).await?;
            let exclude = id.map(|id| subtree_ids(&groups, id)).unwrap_or_default();
            let page = FormTemplate {
                layout: user.layout(),
                heading: if id.is_some() {
                    "Gruppe bearbeiten"
                } else {
                    "Neue Gruppe"
                },
                action: id.map_or_else(|| "/groups".to_owned(), |id| format!("/groups/{id}")),
                delete_action: id.map(|id| format!("/groups/{id}/delete")),
                error: Some(message),
                name: form.name.clone(),
                parents: group_options(
                    &groups,
                    parse_optional_id(&form.parent_id).ok().flatten(),
                    &exclude,
                ),
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
        return Err(Error::Invalid("Bitte das Löschen bestätigen.".to_owned()));
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
        assert!(home.body.contains("indent-1"));
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
