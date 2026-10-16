//! Web layer: axum router, shared state and handlers.

mod account;
mod assets;
pub mod auth;
pub mod credentials;
pub mod devices;
pub mod groups;
pub mod mail_settings;
pub mod mibs;
pub mod order;
pub mod overview;
pub mod sensor_detail;
pub mod sensor_text;
pub mod sensor_values;
pub mod sensors;
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
        .route("/groups/{id}", post(groups::update))
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
        .route("/users", get(users::list).post(users::create))
        .route("/users/{id}", get(users::edit))
        .route("/users/{id}/role", post(users::set_role))
        .route("/users/{id}/password", post(users::set_password))
        .route("/users/{id}/delete", post(users::delete))
        .route("/templates", get(templates::list))
        .route(
            "/templates/{id}",
            get(templates::detail).post(templates::update),
        )
        .route("/templates/{id}/delete", post(templates::delete))
        .route("/sensors/{id}/rename", post(sensor_detail::rename))
        .route("/sensors/{id}/states", post(sensor_detail::save_states))
        .route("/sensors/{id}/toggle", post(sensors::toggle))
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
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

fn security_header(name: HeaderName, value: &'static str) -> SetResponseHeaderLayer<HeaderValue> {
    SetResponseHeaderLayer::if_not_present(name, HeaderValue::from_static(value))
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
        crate::error::Error::Invalid(
            "Für dieses Gerät ist kein SNMP eingerichtet (nur Ping). Ein SNMP-Profil lässt sich unter „Gerät bearbeiten“ wählen."
                .to_owned(),
        )
    })?;
    crate::storage::credentials::load_credentials(&state.pool, &state.secrets, profile)
        .await?
        .ok_or_else(|| {
            crate::error::Error::Internal("device refers to a missing profile".to_owned())
        })
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
pub fn flash_message(key: Option<&str>) -> Option<&'static str> {
    match key? {
        "group_created" => Some("Gruppe angelegt."),
        "group_updated" => Some("Gruppe gespeichert."),
        "group_deleted" => Some("Gruppe gelöscht."),
        "device_created" => Some("Gerät angelegt."),
        "device_created_ping" => Some("Gerät ohne SNMP angelegt; der Ping-Sensor ist aktiv."),
        "device_updated" => Some("Gerät gespeichert."),
        "device_deleted" => Some("Gerät gelöscht."),
        "sensor_created" => Some("Sensor angelegt."),
        "sensors_added" => Some("Sensoren angelegt."),
        "sensor_deleted" => Some("Sensor gelöscht."),
        "sensor_paused" => Some("Sensor deaktiviert."),
        "sensor_resumed" => Some("Sensor aktiviert."),
        "values_added_inactive" => Some(
            "Sensoren angelegt – sie sind noch inaktiv, unten unter „Inaktive Sensoren“ aktivieren.",
        ),
        "values_added_active" => Some("Sensoren angelegt und aktiviert."),
        "sensors_activated" => Some("Sensoren aktiviert."),
        "sensors_deactivated" => Some("Sensoren deaktiviert."),
        "sensors_deleted" => Some("Sensoren gelöscht."),
        "thresholds_saved" => Some("Schwellwerte gespeichert."),
        "sensor_renamed" => Some("Sensor umbenannt."),
        "states_saved" => {
            Some("Bewertung der Statuswerte gespeichert. Sie gilt ab der nächsten Abfrage.")
        }
        "states_reset" => Some("Bewertung der Statuswerte entfernt."),
        "order_saved" => Some("Reihenfolge gespeichert."),
        "template_created" => Some(
            "Vorlage gespeichert. Sie lässt sich beim Anlegen eines Geräts oder mit „Vorlage anwenden“ nutzen.",
        ),
        "template_saved" => Some("Vorlage gespeichert."),
        "user_created" => Some("Benutzer angelegt."),
        "account_password_saved" => {
            Some("Passwort geändert. Deine anderen Sitzungen wurden abgemeldet.")
        }
        "user_role_saved" => Some("Rolle gespeichert."),
        "user_password_saved" => Some("Passwort gesetzt."),
        "user_deleted" => Some("Benutzer gelöscht."),
        "template_deleted" => Some("Vorlage gelöscht."),
        "order_reset" => Some("Reihenfolge zurückgesetzt: wieder automatisch sortiert."),
        "mail_saved" => Some("E-Mail-Einstellungen gespeichert."),
        "mib_deleted" => Some("MIB-Datei gelöscht."),
        _ => None,
    }
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

/// Data every logged-in page needs for the header (navigation, user, logout form).
pub struct Layout {
    pub username: String,
    pub role_label: &'static str,
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
