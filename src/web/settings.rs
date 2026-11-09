//! The settings area (administrators only): `/settings` opens its first page, `/settings/updates` shows
//! the installed version, what the update check found and how an update is installed. The other pages of
//! the area keep their own addresses (see [`crate::web::SETTINGS`]); the frame shows their navigation.

use askama::Template;
use axum::extract::State;
use axum::response::{Html, Redirect};

use crate::error::Result;
use crate::i18n::Lang;
use crate::update_check::Release;
use crate::web::auth::CurrentUser;
use crate::web::{AppState, Layout, SETTINGS, time_html};

/// The first page of the settings area.
pub async fn index(user: CurrentUser) -> Result<Redirect> {
    user.require_admin()?;
    Ok(Redirect::to(SETTINGS[0].1))
}

#[derive(Template)]
#[template(path = "settings_updates.html")]
struct UpdatesTemplate {
    /// Language of the page.
    t: Lang,
    layout: Layout,
    /// The newer release, if the last check found one.
    release: Option<Release>,
    /// Whether the update check runs (config `update_check`).
    check_on: bool,
    /// When the last check got an answer, as `<time>` (rendered with `|safe`).
    checked: Option<String>,
    /// The package file of the newer release, or a placeholder.
    package: String,
}

pub async fn updates(State(state): State<AppState>, user: CurrentUser) -> Result<Html<String>> {
    user.require_admin()?;
    let release = state.latest_release.get();
    let package = format!(
        "opennetmon_{}-1_amd64.deb",
        release.as_ref().map_or("<version>", |r| r.version.as_str())
    );
    let page = UpdatesTemplate {
        layout: user.layout(),
        t: user.lang,
        release,
        check_on: state.update_check,
        checked: state.latest_release.checked_ms().map(time_html),
        package,
    };
    Ok(Html(page.render()?))
}
