//! E-mail settings page (admin only): server, sender, recipients, a test mail and the delivery log.

use std::time::{SystemTime, UNIX_EPOCH};

use askama::Template;
use axum::Form;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::credentials::Secret;
use crate::error::Result;
use crate::i18n::{Lang, Msg};
use crate::mail::{self, MailInput, Security};
use crate::storage::mail::{self as store, Kind, MailView};
use crate::web::auth::{CsrfForm, CurrentUser};
use crate::web::overview::FlashQuery;
use crate::web::{AppState, Layout, flash_message, time_html};

const LOG_ENTRIES: u32 = 20;

/// Values shown in the form (the password never appears, only whether one is stored).
struct FormValues {
    enabled: bool,
    host: String,
    port: String,
    security: Security,
    username: String,
    password_set: bool,
    from_address: String,
    recipients: String,
    notify_warnings: bool,
}

impl From<MailView> for FormValues {
    fn from(v: MailView) -> Self {
        Self {
            enabled: v.enabled,
            host: v.host,
            port: v.port.to_string(),
            security: v.security,
            username: v.username,
            password_set: v.password_set,
            from_address: v.from_address,
            recipients: v.recipients,
            notify_warnings: v.notify_warnings,
        }
    }
}

struct SecurityOption {
    value: &'static str,
    label: String,
    selected: bool,
}

struct LogRow {
    at: String,
    kind: String,
    subject: String,
    recipients: String,
    ok: bool,
    error: String,
}

/// Result of a test mail, shown on the page.
enum TestOutcome {
    Sent(String),
    Failed(String),
}

#[derive(Template)]
#[template(path = "mail_settings.html")]
struct PageTemplate {
    /// Language of the page.
    t: Lang,
    layout: Layout,
    form: FormValues,
    securities: Vec<SecurityOption>,
    log: Vec<LogRow>,
    flash: Option<String>,
    error: Option<String>,
    test_ok: Option<String>,
    test_error: Option<String>,
}

/// Deliberately no `Debug`: the form carries a password.
#[derive(Deserialize)]
pub struct MailForm {
    csrf_token: String,
    enabled: Option<String>,
    #[serde(default)]
    host: String,
    port: String,
    security: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    from_address: String,
    #[serde(default)]
    recipients: String,
    notify_warnings: Option<String>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

impl MailForm {
    /// What the user typed, to show again after an error (without the password).
    fn values(&self, password_set: bool) -> FormValues {
        FormValues {
            enabled: self.enabled.is_some(),
            host: self.host.clone(),
            port: self.port.clone(),
            security: Security::parse(&self.security).unwrap_or(Security::StartTls),
            username: self.username.clone(),
            password_set,
            from_address: self.from_address.clone(),
            recipients: self.recipients.clone(),
            notify_warnings: self.notify_warnings.is_some(),
        }
    }

    fn to_input(&self) -> std::result::Result<MailInput, Msg> {
        let port = self
            .port
            .trim()
            .parse::<u16>()
            .map_err(|_| Msg::new("mail-port-invalid"))?;
        let security =
            Security::parse(&self.security).ok_or_else(|| Msg::new("mail-security-missing"))?;
        Ok(MailInput {
            enabled: self.enabled.is_some(),
            host: self.host.clone(),
            port,
            security,
            username: self.username.clone(),
            password: (!self.password.is_empty()).then(|| Secret::new(self.password.clone())),
            from_address: self.from_address.clone(),
            recipients: self.recipients.clone(),
            notify_warnings: self.notify_warnings.is_some(),
        })
    }
}

struct Extras {
    flash: Option<String>,
    error: Option<String>,
    test: Option<TestOutcome>,
}

async fn render(
    state: &AppState,
    user: &CurrentUser,
    form: FormValues,
    extras: Extras,
) -> Result<String> {
    let log = store::recent_notifications(&state.pool, LOG_ENTRIES)
        .await?
        .into_iter()
        .map(|n| LogRow {
            at: time_html(n.ts_ms),
            kind: user.lang.get(n.kind.label_id()),
            subject: n.subject,
            recipients: n.recipients,
            ok: n.ok,
            error: user.lang.msg(&n.error),
        })
        .collect();
    let securities = [Security::StartTls, Security::Tls, Security::None]
        .into_iter()
        .map(|s| SecurityOption {
            value: s.as_str(),
            label: user.lang.get(s.label_id()),
            selected: s == form.security,
        })
        .collect();
    let (test_ok, test_error) = match extras.test {
        Some(TestOutcome::Sent(m)) => (Some(m), None),
        Some(TestOutcome::Failed(m)) => (None, Some(m)),
        None => (None, None),
    };
    let page = PageTemplate {
        layout: user.layout(),
        t: user.lang,
        form,
        securities,
        log,
        flash: extras.flash,
        error: extras.error,
        test_ok,
        test_error,
    };
    Ok(page.render()?)
}

pub async fn show(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(query): Query<FlashQuery>,
) -> Result<Html<String>> {
    user.require_admin()?;
    let view = store::get_view(&state.pool).await?;
    Ok(Html(
        render(
            &state,
            &user,
            view.into(),
            Extras {
                flash: flash_message(user.lang, query.msg.as_deref()),
                error: None,
                test: None,
            },
        )
        .await?,
    ))
}

pub async fn save(
    State(state): State<AppState>,
    user: CurrentUser,
    Form(form): Form<MailForm>,
) -> Result<Response> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    let checked = form.to_input().and_then(|input| input.check());
    match checked {
        Ok(checked) => {
            store::save(&state.pool, &state.secrets, &checked).await?;
            tracing::info!(user = %user.username, enabled = checked.enabled, "mail settings saved");
            Ok(Redirect::to("/settings/mail?msg=mail_saved").into_response())
        }
        Err(message) => {
            let password_set = store::get_view(&state.pool).await?.password_set;
            let html = render(
                &state,
                &user,
                form.values(password_set),
                Extras {
                    flash: None,
                    error: Some(user.lang.msg(&message)),
                    test: None,
                },
            )
            .await?;
            Ok((StatusCode::UNPROCESSABLE_ENTITY, Html(html)).into_response())
        }
    }
}

/// Sends a test mail with the *stored* settings (works while notifications are still switched off).
pub async fn send_test(
    State(state): State<AppState>,
    user: CurrentUser,
    Form(form): Form<CsrfForm>,
) -> Result<Html<String>> {
    user.require_admin()?;
    user.verify_csrf(&form.csrf_token)?;
    let config = store::load_config(&state.pool, &state.secrets).await?;
    // Mail texts stay German until the mail language can be chosen (PLAN phase 9, step 3).
    let mail_lang = Lang::De;
    let subject = format!("[opennetmon] {}", mail_lang.get("mail-test-subject"));
    let body = format!("{}\n", mail_lang.get("mail-test-body"));
    let outcome = mail::send(&config, &subject, &body).await;
    let error = outcome.as_ref().err().map(mail::MailError::user_message);
    store::record_notification(
        &state.pool,
        now_ms(),
        Kind::Test,
        &subject,
        &config.recipients,
        error.as_ref(),
    )
    .await?;
    tracing::info!(user = %user.username, ok = error.is_none(), "test mail");
    let test = match error {
        None => TestOutcome::Sent(user.lang.get1(
            "mail-test-sent",
            "recipients",
            config.recipients.join(", "),
        )),
        Some(message) => TestOutcome::Failed(user.lang.msg(&message)),
    };
    let view = store::get_view(&state.pool).await?;
    Ok(Html(
        render(
            &state,
            &user,
            view.into(),
            Extras {
                flash: None,
                error: None,
                test: Some(test),
            },
        )
        .await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::testsmtp::{Behavior, TestSmtp};
    use crate::web::testutil::*;

    fn form(csrf: &str, port: u16, extra: &str) -> String {
        format!(
            "csrf_token={csrf}&enabled=on&notify_warnings=on&host=127.0.0.1&port={port}&security=none\
             &username=&password=&from_address=opennetmon%40example.org&recipients=admin%40example.org%0Aops%40example.org{extra}"
        )
    }

    #[tokio::test]
    async fn only_admins_may_see_or_change_the_settings() {
        let h = harness(true).await;
        assert_eq!(
            send(&h.app, get("/settings/mail", None)).await.status,
            StatusCode::SEE_OTHER
        );
        let (viewer, vcsrf) = login(&h, "bert").await;
        assert_eq!(
            send(&h.app, get("/settings/mail", Some(&viewer)))
                .await
                .status,
            StatusCode::FORBIDDEN
        );
        for (path, body) in [
            ("/settings/mail", form(&vcsrf, 25, "")),
            ("/settings/mail/test", format!("csrf_token={vcsrf}")),
        ] {
            assert_eq!(
                send(&h.app, post(path, Some(&viewer), &body)).await.status,
                StatusCode::FORBIDDEN,
                "{path}"
            );
        }
        let home = send(&h.app, get("/", Some(&viewer))).await;
        assert!(
            !home.body.contains("/settings/mail"),
            "viewers get no menu entry"
        );

        let (admin, _) = login(&h, "anna").await;
        for (path, body) in [
            ("/settings/mail", form("wrong", 25, "")),
            ("/settings/mail/test", "csrf_token=wrong".to_owned()),
        ] {
            assert_eq!(
                send(&h.app, post(path, Some(&admin), &body)).await.status,
                StatusCode::FORBIDDEN,
                "{path}"
            );
        }
        let home = send(&h.app, get("/", Some(&admin))).await;
        assert!(home.body.contains("href=\"/settings/mail\""));
        assert!(
            !crate::storage::mail::get_view(&h.state.pool)
                .await
                .unwrap()
                .enabled
        );
    }

    #[tokio::test]
    async fn settings_are_saved_shown_and_the_password_stays_hidden() {
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;
        let body = form(&csrf, 2525, "").replace(
            "username=&password=",
            "username=opennetmon&password=pw-sehr-geheim",
        );
        let reply = send(&h.app, post("/settings/mail", Some(&admin), &body)).await;
        assert_eq!(reply.location(), "/settings/mail?msg=mail_saved");

        let stored = crate::storage::mail::load_config(&h.state.pool, &h.state.secrets)
            .await
            .unwrap();
        assert_eq!(
            (stored.host.as_str(), stored.port, stored.security),
            ("127.0.0.1", 2525, Security::None)
        );
        assert_eq!(stored.recipients, ["admin@example.org", "ops@example.org"]);
        assert_eq!(
            stored.password.as_ref().map(Secret::expose),
            Some("pw-sehr-geheim")
        );

        let page = send(&h.app, get("/settings/mail?msg=mail_saved", Some(&admin))).await;
        assert!(page.body.contains("E-Mail-Einstellungen gespeichert."));
        assert!(
            page.body.contains("value=\"127.0.0.1\"")
                && page.body.contains("value=\"2525\"")
                && page.body.contains("value=\"opennetmon\"")
        );
        assert!(page.body.contains("admin@example.org\nops@example.org"));
        assert!(page.body.contains("gesetzt – leer lassen zum Beibehalten"));
        assert!(
            !page.body.contains("pw-sehr-geheim"),
            "the password is never rendered"
        );

        // An empty password field keeps the stored password.
        let again = form(&csrf, 2525, "").replace("username=&", "username=opennetmon&");
        send(&h.app, post("/settings/mail", Some(&admin), &again)).await;
        let kept = crate::storage::mail::load_config(&h.state.pool, &h.state.secrets)
            .await
            .unwrap();
        assert_eq!(
            kept.password.as_ref().map(Secret::expose),
            Some("pw-sehr-geheim")
        );
    }

    #[tokio::test]
    async fn invalid_settings_are_explained_and_nothing_changes() {
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;
        for (body, expected) in [
            (
                form(&csrf, 25, "").replace(
                    "recipients=admin%40example.org%0Aops%40example.org",
                    "recipients=kaputt",
                ),
                "keine gültige E-Mail-Adresse",
            ),
            (
                form(&csrf, 25, "").replace("host=127.0.0.1", "host="),
                "SMTP-Server",
            ),
            (form(&csrf, 25, "").replace("port=25", "port=abc"), "Port"),
            (form(&csrf, 25, "").replace("port=25", "port=70000"), "Port"),
            (
                form(&csrf, 25, "").replace("security=none", "security=ssl"),
                "Verschlüsselung",
            ),
        ] {
            let reply = send(&h.app, post("/settings/mail", Some(&admin), &body)).await;
            assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
            assert!(reply.body.contains(expected), "{expected}: {}", reply.body);
        }
        let bad = form(&csrf, 25, "")
            .replace("username=&password=", "username=u&password=geheim-1")
            .replace("host=127.0.0.1", "host=");
        let reply = send(&h.app, post("/settings/mail", Some(&admin), &bad)).await;
        assert!(
            reply.body.contains("value=\"u\"") && !reply.body.contains("geheim-1"),
            "input is echoed, the password is not"
        );
        let view = crate::storage::mail::get_view(&h.state.pool).await.unwrap();
        assert!(!view.enabled && view.host.is_empty());
    }

    #[tokio::test]
    async fn a_half_finished_configuration_can_be_saved_while_switched_off() {
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;
        let body = "csrf_token=".to_owned()
            + &csrf
            + "&host=mail.example.org&port=587&security=starttls&username=&password=&from_address=&recipients=";
        let reply = send(&h.app, post("/settings/mail", Some(&admin), &body)).await;
        assert_eq!(reply.status, StatusCode::SEE_OTHER);
        assert_eq!(
            crate::storage::mail::get_view(&h.state.pool)
                .await
                .unwrap()
                .host,
            "mail.example.org"
        );
    }

    #[tokio::test]
    async fn the_test_mail_uses_the_stored_settings_and_is_logged() {
        let smtp = TestSmtp::spawn(Behavior::default()).await;
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;
        // Switched off, but fully configured: testing still works.
        let body = form(&csrf, smtp.port, "").replace("enabled=on&", "");
        send(&h.app, post("/settings/mail", Some(&admin), &body)).await;
        let page = send(
            &h.app,
            post(
                "/settings/mail/test",
                Some(&admin),
                &format!("csrf_token={csrf}"),
            ),
        )
        .await;
        assert_eq!(page.status, StatusCode::OK);
        assert!(
            page.body
                .contains("Test-E-Mail an admin@example.org, ops@example.org gesendet."),
            "{}",
            page.body
        );
        assert_eq!(smtp.received().len(), 1);
        assert!(
            smtp.received()[0]
                .data
                .contains("Subject: [opennetmon] Test-E-Mail")
        );
        assert!(page.body.contains("gesendet</span>") && page.body.contains("Test"));
    }

    #[tokio::test]
    async fn a_failing_test_mail_shows_the_reason() {
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;
        let page = send(
            &h.app,
            post(
                "/settings/mail/test",
                Some(&admin),
                &format!("csrf_token={csrf}"),
            ),
        )
        .await;
        assert_eq!(page.status, StatusCode::OK);
        assert!(page.body.contains("kein SMTP-Server"), "{}", page.body);

        let broken = TestSmtp::spawn(Behavior {
            reject_recipients: true,
            ..Behavior::default()
        })
        .await;
        send(
            &h.app,
            post(
                "/settings/mail",
                Some(&admin),
                &form(&csrf, broken.port, ""),
            ),
        )
        .await;
        let page = send(
            &h.app,
            post(
                "/settings/mail/test",
                Some(&admin),
                &format!("csrf_token={csrf}"),
            ),
        )
        .await;
        assert!(
            page.body.contains("Versand fehlgeschlagen"),
            "{}",
            page.body
        );
        assert!(page.body.contains("Fehler</span>"));
    }

    #[tokio::test]
    async fn the_page_and_its_errors_are_shown_in_english() {
        let h = harness(true).await;
        crate::storage::users::set_language(&h.state.pool, 1, Some(Lang::En))
            .await
            .unwrap();
        let (admin, csrf) = login(&h, "anna").await;
        let page = send(&h.app, get("/settings/mail", Some(&admin))).await;
        for text in [
            "<h1>E-mail notification</h1>",
            "No encryption",
            "No e-mails sent yet.",
        ] {
            assert!(page.body.contains(text), "{text} missing");
        }
        let form = format!(
            "csrf_token={csrf}&enabled=on&host=mail.example.org&port=587&security=starttls&from_address=nope&recipients=a%40example.org"
        );
        let refused = send(&h.app, post("/settings/mail", Some(&admin), &form)).await;
        assert_eq!(refused.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            refused
                .body
                .contains("“nope” is not a valid e-mail address.")
        );
    }
}
