//! Web layer: axum router, shared state and handlers.

mod account;
mod assets;
pub mod auth;
pub mod credentials;
pub mod devices;
pub mod groups;
pub mod log;
pub mod mail_settings;
pub mod mibs;
pub mod order;
pub mod overview;
pub mod pause;
pub mod sensor_detail;
pub mod sensor_text;
pub mod sensor_values;
pub mod sensors;
pub mod server_values;
pub mod sparkline;
pub mod status_list;
pub mod templates;
#[cfg(test)]
pub(crate) mod testutil;
pub mod users;

use axum::Router;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum::routing::{get, post};
use sqlx::SqlitePool;
use tower_http::compression::CompressionLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::TraceLayer;

use crate::config::Config;
use crate::crypto::SecretBox;
use crate::i18n::Lang;
use crate::storage;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub secrets: Arc<SecretBox>,
    pub cookie_secure: bool,
    pub session_ttl_secs: i64,
    pub login_throttle: Arc<crate::auth::throttle::LoginThrottle>,
    pub client_ip_header: Option<crate::auth::throttle::ClientIpHeader>,
    /// Bundled plus uploaded MIBs; replaced as a whole after every upload or deletion.
    mibs: Arc<std::sync::RwLock<Arc<crate::mib::MibIndex>>>,
}

impl AppState {
    pub fn new(pool: SqlitePool, config: &Config, secrets: Arc<SecretBox>) -> Self {
        Self {
            pool,
            secrets,
            cookie_secure: config.cookie_secure,
            login_throttle: Arc::default(),
            client_ip_header: config.client_ip_header,
            mibs: Arc::new(std::sync::RwLock::new(crate::mib::bundled_index())),
            session_ttl_secs: i64::try_from(config.session_ttl_hours.saturating_mul(3600))
                .unwrap_or(i64::MAX),
        }
    }

    /// The current MIB index (cheap: shares the index).
    pub fn mib_index(&self) -> Arc<crate::mib::MibIndex> {
        match self.mibs.read() {
            Ok(guard) => Arc::clone(&guard),
            Err(poisoned) => Arc::clone(&poisoned.into_inner()),
        }
    }

    /// Rebuilds the MIB index from the bundled MIBs and all uploaded files.
    pub async fn reload_mibs(&self) -> crate::error::Result<()> {
        let contents = storage::mibs::all_contents(&self.pool).await?;
        let index = tokio::task::spawn_blocking(move || {
            let sources: Vec<crate::mib::Source<'_>> = contents
                .iter()
                .map(|(id, text)| crate::mib::Source {
                    origin: crate::mib::Origin::Imported(*id),
                    text,
                })
                .collect();
            crate::mib::MibIndex::build(&sources)
        })
        .await
        .map_err(|e| crate::error::Error::Internal(format!("MIB index task failed: {e}")))?;
        let index = Arc::new(index);
        match self.mibs.write() {
            Ok(mut guard) => *guard = index,
            Err(poisoned) => *poisoned.into_inner() = index,
        }
        Ok(())
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(overview::home))
        .route("/login", get(auth::login_form).post(auth::login_submit))
        .route("/logout", post(auth::logout))
        .route(
            "/credentials",
            get(credentials::list).post(credentials::create),
        )
        .route("/credentials/new", get(credentials::new_form))
        .route("/credentials/{id}/edit", get(credentials::edit_form))
        .route("/credentials/{id}", post(credentials::update))
        .route("/credentials/{id}/delete", post(credentials::delete))
        .route("/groups/new", get(groups::new_form))
        .route("/groups", post(groups::create))
        .route("/groups/{id}/edit", get(groups::edit_form))
        .route("/groups/{id}", get(overview::group).post(groups::update))
        .route("/groups/{id}/delete", post(groups::delete))
        .route(
            "/groups/{id}/order",
            get(order::group_page).post(order::save_group),
        )
        .route("/devices/new", get(devices::new_form))
        .route("/devices", post(devices::create))
        .route("/devices/{id}/edit", get(devices::edit_form))
        .route("/devices/{id}", post(devices::update))
        .route("/devices/{id}/delete", post(devices::delete))
        .route("/devices/{id}/test", post(devices::test_connection))
        .route(
            "/devices/{id}/sensors",
            get(sensors::list).post(sensors::create),
        )
        .route("/devices/{id}/discover", post(sensors::discover))
        .route("/devices/{id}/values", post(sensor_values::discover))
        .route(
            "/devices/{id}/server",
            get(server_values::page).post(server_values::create),
        )
        .route("/devices/{id}/values/create", post(sensor_values::create))
        .route("/devices/{id}/sensors/bulk", post(sensors::bulk))
        .route(
            "/devices/{id}/sensors/order",
            get(order::sensor_page).post(order::save_sensors),
        )
        .route(
            "/devices/{id}/sensors/interfaces",
            post(sensors::create_interfaces),
        )
        .route("/status/{filter}", get(status_list::list))
        .route("/log", get(log::log))
        .route("/sensors/{id}", get(sensor_detail::detail))
        .route("/sensors/{id}/data", get(sensor_detail::data))
        .route(
            "/sensors/{id}/thresholds",
            post(sensor_detail::save_thresholds),
        )
        .route("/devices/{id}/template/new", get(templates::new_form))
        .route("/devices/{id}/template", post(templates::create))
        .route(
            "/devices/{id}/template/apply",
            get(templates::apply_form).post(templates::apply),
        )
        .route("/account", get(account::show))
        .route("/account/password", post(account::change_password))
        .route("/account/language", post(account::set_language))
        .route("/users", get(users::list).post(users::create))
        .route("/users/{id}", get(users::edit))
        .route("/users/{id}/role", post(users::set_role))
        .route("/users/{id}/password", post(users::set_password))
        .route("/users/{id}/language", post(users::set_language))
        .route("/users/{id}/delete", post(users::delete))
        .route("/templates", get(templates::list))
        .route(
            "/templates/{id}",
            get(templates::detail).post(templates::update),
        )
        .route("/templates/{id}/delete", post(templates::delete))
        .route("/sensors/{id}/rename", post(sensor_detail::rename))
        .route("/sensors/{id}/states", post(sensor_detail::save_states))
        .route("/pause/{kind}/{id}", get(pause::form).post(pause::pause))
        .route("/resume/{kind}/{id}", post(pause::resume))
        .route("/sensors/{id}/delete", post(sensors::delete))
        .route(
            "/settings/mail",
            get(mail_settings::show).post(mail_settings::save),
        )
        .route("/settings/mail/test", post(mail_settings::send_test))
        .route("/mibs", get(mibs::show))
        .route(
            "/mibs/upload",
            post(mibs::upload).layer(axum::extract::DefaultBodyLimit::max(mibs::MAX_UPLOAD_BYTES)),
        )
        .route("/mibs/files/{id}/delete", post(mibs::delete))
        .route("/mibs/search.json", get(mibs::search_json))
        .route("/healthz", get(healthz))
        .route("/static/{*path}", get(assets::serve))
        .layer(security_header(header::X_CONTENT_TYPE_OPTIONS, "nosniff"))
        .layer(security_header(header::X_FRAME_OPTIONS, "DENY"))
        .layer(security_header(header::REFERRER_POLICY, "same-origin"))
        .layer(security_header(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'self'; frame-ancestors 'none'; form-action 'self'",
        ))
        .layer(axum::middleware::from_fn(request_language))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Runs every request in the browser's language; signed-in users switch it to their own (see `CurrentUser`).
async fn request_language(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let lang = auth::browser_language(request.headers());
    crate::i18n::scope(lang, next.run(request)).await
}

fn security_header(name: HeaderName, value: &'static str) -> SetResponseHeaderLayer<HeaderValue> {
    SetResponseHeaderLayer::if_not_present(name, HeaderValue::from_static(value))
}

/// An entry of the language selection; `code` is empty for "follow the browser".
pub struct LanguageOption {
    pub code: &'static str,
    pub label: String,
    pub selected: bool,
}

/// The choices for a user's language: the browser's first, then every language by its own name.
pub fn language_options(t: Lang, chosen: Option<Lang>) -> Vec<LanguageOption> {
    let mut options = vec![LanguageOption {
        code: "",
        label: t.get("language-browser"),
        selected: chosen.is_none(),
    }];
    options.extend(Lang::ALL.into_iter().map(|lang| LanguageOption {
        code: lang.code(),
        label: lang.native_name().to_owned(),
        selected: chosen == Some(lang),
    }));
    options
}

/// A submitted language choice: empty for "follow the browser".
pub fn parse_language_choice(code: &str) -> crate::error::Result<Option<Lang>> {
    if code.is_empty() {
        return Ok(None);
    }
    Lang::parse(code)
        .map(Some)
        .ok_or_else(|| crate::error::Error::invalid(crate::i18n::Msg::new("language-invalid")))
}

/// How a user's language choice is shown in lists.
pub fn language_label(t: Lang, chosen: Option<Lang>) -> String {
    chosen.map_or_else(|| t.get("language-browser"), |l| l.native_name().to_owned())
}

/// An entry of a `<select>` for groups or profiles.
pub struct SelectOption {
    pub id: i64,
    pub label: String,
    pub selected: bool,
}

/// The SNMP credentials of a device for an interactive action; a user-facing error for ping-only
/// devices.
pub(crate) async fn snmp_credentials(
    state: &AppState,
    device: &crate::devices::Device,
) -> crate::error::Result<crate::credentials::Credentials> {
    let profile = device.profile_id.ok_or_else(|| {
        crate::error::Error::invalid(crate::i18n::Msg::new("device-without-snmp"))
    })?;
    crate::storage::credentials::load_credentials(&state.pool, &state.secrets, profile)
        .await?
        .ok_or_else(|| {
            crate::error::Error::Internal("device refers to a missing profile".to_owned())
        })
}

/// A group in the breadcrumb trail of a device or group page, linking to the group's page.
#[derive(Clone)]
pub struct Crumb {
    pub name: String,
    pub href: String,
}

/// The groups from the top level down to `group_id` (empty for a device without a group).
pub(crate) async fn group_trail(
    pool: &SqlitePool,
    group_id: Option<i64>,
) -> crate::error::Result<Vec<Crumb>> {
    let Some(mut id) = group_id else {
        return Ok(Vec::new());
    };
    let groups = storage::groups::list_groups(pool).await?;
    let mut trail = Vec::new();
    // Bounded by the number of groups, so a broken parent chain cannot loop.
    while let Some(group) = groups.iter().find(|g| g.id == id) {
        trail.push(Crumb {
            name: group.name.clone(),
            href: format!("/groups/{}", group.id),
        });
        match group.parent_id {
            Some(parent) if trail.len() < groups.len() => id = parent,
            _ => break,
        }
    }
    trail.reverse();
    Ok(trail)
}

/// The group page a form was opened from (`back=group-<id>`), so that it returns there.
pub(crate) fn back_to_group(back: &str) -> Option<i64> {
    back.strip_prefix("group-")?.parse().ok()
}

/// Group options in tree order, indented by depth. `exclude` hides groups (e.g. a group and its subtree).
pub fn group_options(
    groups: &[crate::devices::Group],
    selected: Option<i64>,
    exclude: &[i64],
) -> Vec<SelectOption> {
    crate::devices::flatten_groups(groups)
        .into_iter()
        .filter(|n| !exclude.contains(&n.group.id))
        .map(|n| SelectOption {
            id: n.group.id,
            label: format!(
                "{}{}",
                "\u{00a0}\u{00a0}\u{00a0}".repeat(n.depth - 1),
                n.group.name
            ),
            selected: selected == Some(n.group.id),
        })
        .collect()
}

/// Message shown after a successful action (`?msg=` is mapped to fixed texts, never echoed).
pub fn flash_message(t: Lang, key: Option<&str>) -> Option<String> {
    let id = match key? {
        "group_created" => "flash-group-created",
        "group_updated" => "flash-group-updated",
        "group_deleted" => "flash-group-deleted",
        "device_created" => "flash-device-created",
        "device_created_ping" => "flash-device-created-ping",
        "device_updated" => "flash-device-updated",
        "device_deleted" => "flash-device-deleted",
        "sensor_created" => "flash-sensor-created",
        "sensors_added" => "flash-sensors-added",
        "sensor_deleted" => "flash-sensor-deleted",
        "sensor_paused" => "flash-sensor-paused",
        "sensor_resumed" => "flash-sensor-resumed",
        "device_paused" => "flash-device-paused",
        "device_resumed" => "flash-device-resumed",
        "group_paused" => "flash-group-paused",
        "group_resumed" => "flash-group-resumed",
        "values_added_inactive" => "flash-values-added-inactive",
        "values_added_active" => "flash-values-added-active",
        "sensors_activated" => "flash-sensors-activated",
        "sensors_deleted" => "flash-sensors-deleted",
        "thresholds_saved" => "flash-thresholds-saved",
        "sensor_renamed" => "flash-sensor-renamed",
        "states_saved" => "flash-states-saved",
        "states_reset" => "flash-states-reset",
        "order_saved" => "flash-order-saved",
        "template_created" => "flash-template-created",
        "template_saved" => "flash-template-saved",
        "user_created" => "flash-user-created",
        "account_password_saved" => "flash-account-password-saved",
        "user_role_saved" => "flash-user-role-saved",
        "user_password_saved" => "flash-user-password-saved",
        "user_language_saved" => "flash-user-language-saved",
        "user_deleted" => "flash-user-deleted",
        "template_deleted" => "flash-template-deleted",
        "order_reset" => "flash-order-reset",
        "mail_saved" => "flash-mail-saved",
        "language_saved" => "flash-language-saved",
        "mib_deleted" => "flash-mib-deleted",
        _ => return None,
    };
    Some(t.get(id))
}

/// Civil date and time of Unix milliseconds in UTC: `(year, month, day, hour, minute, second)`.
fn civil_utc(ms: i64) -> (i64, i64, i64, i64, i64, i64) {
    let secs = ms.div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day, rem / 3_600, (rem % 3_600) / 60, rem % 60)
}

/// Formats Unix milliseconds as `2026-10-15 19:32:10 UTC` (plain text, e.g. for e-mails).
pub fn format_utc(ms: i64) -> String {
    let (y, mo, d, h, mi, s) = civil_utc(ms);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02} UTC")
}

/// A timestamp for HTML pages: `<time>` with the exact instant in `datetime` and the UTC text as content.
/// `static/time.js` rewrites it into the browser's time zone; without JavaScript the UTC text stays.
/// The result contains only digits and fixed text, so it may be rendered with `|safe`.
pub fn time_html(ms: i64) -> String {
    let (y, mo, d, h, mi, s) = civil_utc(ms);
    format!(
        "<time class=\"ts\" datetime=\"{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z\">{}</time>",
        format_utc(ms)
    )
}

/// Like [`time_html`] for an ISO timestamp as SQLite writes it (`2026-10-16T18:30:00.123Z`).
/// Anything that is not a plain timestamp is reduced to its digits and separators, so the result
/// is always safe to render with `|safe`.
pub fn iso_time_html(iso: &str) -> String {
    let clean: String = iso
        .chars()
        .filter(|c| c.is_ascii_digit() || matches!(c, '-' | ':' | 'T' | 'Z' | '.'))
        .take(32)
        .collect();
    let text = clean.replace('T', " ").trim_end_matches('Z').to_owned();
    let text = text.split('.').next().unwrap_or_default().to_owned();
    format!("<time class=\"ts\" datetime=\"{clean}\">{text} UTC</time>")
}

/// Where this program's source code is offered to its users (AGPL-3.0, section 13); `repository` in Cargo.toml.
pub const SOURCE_URL: &str = env!("CARGO_PKG_REPOSITORY");
/// Program version shown next to the source link.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Data every logged-in page needs for the header (navigation, user, logout form).
pub struct Layout {
    pub username: String,
    pub role_label: String,
    pub csrf_token: String,
    pub is_admin: bool,
}

impl Layout {
    /// First letter of the user name for the avatar.
    pub fn initial(&self) -> String {
        self.username
            .chars()
            .next()
            .map(|c| c.to_uppercase().collect())
            .unwrap_or_default()
    }
}

/// Unauthenticated health check; exposes no internal details.
async fn healthz(State(state): State<AppState>) -> (StatusCode, &'static str) {
    match storage::ping(&state.pool).await {
        Ok(()) => (StatusCode::OK, "ok"),
        Err(e) => {
            tracing::warn!(error = %e, "health check failed");
            (StatusCode::SERVICE_UNAVAILABLE, "unavailable")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn test_state() -> (AppState, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let pool = storage::connect(&dir.path().join("t.db")).await.unwrap();
        storage::migrate(&pool).await.unwrap();
        let secrets = Arc::new(SecretBox::from_key([1u8; 32]));
        (AppState::new(pool, &Config::default(), secrets), dir)
    }

    async fn get_path(app: Router, path: &str) -> (StatusCode, String) {
        let response = app
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8(body.to_vec()).unwrap())
    }

    #[test]
    fn utc_timestamps_are_formatted() {
        assert_eq!(format_utc(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(format_utc(951_782_400_000), "2000-02-29 00:00:00 UTC");
        assert_eq!(format_utc(1_781_551_930_000), "2026-06-15 19:32:10 UTC");
        assert_eq!(format_utc(-1_000), "1969-12-31 23:59:59 UTC");
    }

    #[test]
    fn html_timestamps_carry_the_instant_and_a_utc_fallback() {
        assert_eq!(
            time_html(1_781_551_930_000),
            "<time class=\"ts\" datetime=\"2026-06-15T19:32:10Z\">2026-06-15 19:32:10 UTC</time>"
        );
        assert!(time_html(0).contains("datetime=\"1970-01-01T00:00:00Z\""));
    }

    #[test]
    fn iso_timestamps_are_rendered_safely() {
        assert_eq!(
            iso_time_html("2026-10-16T18:30:00.123Z"),
            "<time class=\"ts\" datetime=\"2026-10-16T18:30:00.123Z\">2026-10-16 18:30:00 UTC</time>"
        );
        assert!(!iso_time_html("\"><script>alert(1)</script>").contains("script"));
    }

    #[tokio::test]
    async fn healthz_is_ok_without_auth() {
        let (state, _dir) = test_state().await;
        let (status, body) = get_path(router(state), "/healthz").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "ok");
    }

    #[tokio::test]
    async fn healthz_reports_unavailable_when_db_is_closed() {
        let (state, _dir) = test_state().await;
        state.pool.close().await;
        let (status, body) = get_path(router(state), "/healthz").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body, "unavailable");
    }

    #[tokio::test]
    async fn unknown_path_is_404() {
        let (state, _dir) = test_state().await;
        let (status, _) = get_path(router(state), "/nope").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn responses_carry_security_headers() {
        let (state, _dir) = test_state().await;
        let response = router(state)
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let h = response.headers();
        assert_eq!(h[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(h[header::X_FRAME_OPTIONS], "DENY");
        assert!(
            h[header::CONTENT_SECURITY_POLICY]
                .to_str()
                .unwrap()
                .contains("frame-ancestors 'none'")
        );
    }
}
