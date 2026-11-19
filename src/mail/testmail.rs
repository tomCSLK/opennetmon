//! The test mail of the e-mail settings: what is set up (never the user name or password of the
//! SMTP login), in the look of the notifications.

use askama::Template;

use crate::i18n::Lang;
use crate::mail::view::{self, Button, Chip, Clock, Fact, Frame, Head, Links, Tone, filters};
use crate::mail::{Mail, MailConfig};

#[derive(Template)]
#[template(path = "mail/test.html")]
struct TestTemplate {
    t: Lang,
    frame: Frame,
    head: Head,
    facts: Vec<Fact>,
    chips: Vec<Chip>,
    buttons: Vec<Button>,
}

/// "Triggered by **anna** under Settings → E-mail": the sentence in pieces, the user name bold.
fn bold_user(t: Lang, user: &str) -> Vec<(String, bool)> {
    const MARK: &str = "\u{e000}";
    let text = t.get1("mail-test-by", "user", MARK);
    match text.split_once(MARK) {
        Some((before, after)) => vec![
            (before.to_owned(), false),
            (user.to_owned(), true),
            (after.to_owned(), false),
        ],
        None => vec![(t.get1("mail-test-by", "user", user), false)],
    }
}

/// The test mail with the stored settings `config`, sent by the user `by` of the web interface.
pub fn compose(config: &MailConfig, by: &str, clock: &Clock) -> Mail {
    let t = config.language;
    let links = Links::new(config.ui_url.as_deref());
    let subject = format!("[opennetmon] {}", t.get("mail-test-subject"));
    let server = format!("{}:{}", config.host, config.port);
    let login = if config.username.is_some() {
        t.get("mail-test-login-yes")
    } else {
        t.get("mail-test-login-no")
    };
    let security = format!("{}, {login}", t.get(config.security.label_id()));
    let notify = if config.notify_warnings {
        t.get("mail-test-notify-all")
    } else {
        t.get("mail-test-notify-down")
    };
    let recipients = config.recipients.join(", ");
    let facts = vec![
        Fact::new(t.get("mail-test-server"), server.clone(), security.clone()),
        Fact::new(
            t.get("mail-test-recipients"),
            config.recipients.len().to_string(),
            recipients.clone(),
        ),
        Fact::new(t.get("mail-test-notify"), notify.clone(), String::new()),
        Fact::new(
            t.get("mail-test-language"),
            t.native_name().to_owned(),
            String::new(),
        ),
    ];
    let now = clock.now_ms();
    let by_text = t.get1("mail-test-by", "user", by);
    let mut buttons = Vec::new();
    if let Some(links) = &links {
        buttons.push(Button {
            label: t.get("mail-open-settings"),
            href: links.mail_settings(),
            primary: true,
        });
    }
    let page = TestTemplate {
        t,
        frame: Frame {
            lang_code: t.code(),
            title: t.get("mail-test-subject"),
            preheader: t.get("mail-test-pre"),
            band: Some(Tone::Up),
            host: links.as_ref().map(Links::host).unwrap_or_default(),
            footer: view::footer(t, config.recipients.len(), clock),
            settings: None,
        },
        head: Head {
            chip: Chip {
                tone: Tone::Up,
                text: t.get("mail-test-ok"),
            },
            when: clock.day_time(now),
            title: t.get("mail-test-title"),
            path: Vec::new(),
            host: String::new(),
            note: bold_user(t, by),
        },
        facts,
        chips: vec![
            Chip {
                tone: Tone::Down,
                text: t.get("alert-subject-down"),
            },
            Chip {
                tone: Tone::Warning,
                text: t.get("alert-subject-warning"),
            },
            Chip {
                tone: Tone::Up,
                text: t.get("alert-subject-up"),
            },
            Chip {
                tone: Tone::Off,
                text: t.get("status-paused"),
            },
        ],
        buttons,
    };
    let html = match page.render() {
        Ok(html) => Some(html),
        Err(e) => {
            tracing::warn!(error = %e, "rendering the HTML of the test mail failed");
            None
        }
    };
    let mut text = format!(
        "{} – {}\n{} ({}) · {by_text}\n\n",
        t.get("mail-test-subject").to_uppercase(),
        t.get("mail-test-title"),
        clock.day_time(now),
        clock.zone_name(),
    );
    text.push_str(&view::aligned(&[
        (t.get("mail-test-server"), format!("{server} ({security})")),
        (t.get("mail-test-recipients"), recipients),
        (t.get("mail-test-notify"), notify),
        (t.get("mail-test-language"), t.native_name().to_owned()),
    ]));
    if let Some(links) = &links {
        text.push_str(&format!(
            "\n{}: {}\n",
            t.get("mail-open-settings"),
            links.mail_settings()
        ));
    }
    text.push('\n');
    text.push_str(&view::text_footer(t, None));
    let inline = if html.is_some() {
        vec![view::logo()]
    } else {
        Vec::new()
    };
    Mail {
        subject,
        text,
        html,
        inline,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::Secret;
    use crate::mail::Security;

    pub(crate) fn config(language: Lang, ui_url: Option<&str>) -> MailConfig {
        MailConfig {
            enabled: true,
            host: "mail.example.org".to_owned(),
            port: 587,
            security: Security::StartTls,
            username: Some("smtp-user-geheim".to_owned()),
            password: Some(Secret::new("smtp-pw-geheim")),
            from_address: "opennetmon@example.org".to_owned(),
            recipients: vec!["it@example.org".to_owned(), "ops@example.org".to_owned()],
            notify_warnings: true,
            language,
            ui_url: ui_url.map(str::to_owned),
        }
    }

    fn clock(lang: Lang) -> Clock {
        Clock::new(
            lang,
            jiff::tz::TimeZone::get("Europe/Berlin").unwrap(),
            1_795_095_120_000,
        )
    }

    #[test]
    fn the_test_mail_shows_the_settings_but_never_the_login() {
        let mail = compose(
            &config(Lang::De, Some("https://onm.example.org")),
            "anna",
            &clock(Lang::De),
        );
        assert_eq!(mail.subject, "[opennetmon] Test-E-Mail");
        let html = mail.html.as_deref().unwrap();
        for part in [html, mail.text.as_str()] {
            assert!(part.contains("mail.example.org:587"), "{part}");
            assert!(part.contains("STARTTLS, mit Anmeldung"));
            assert!(part.contains("it@example.org, ops@example.org"));
            assert!(part.contains("Ausfall, Warnung, Wiederherstellung"));
            assert!(part.contains("Do, 19.11.2026 · 14:32"));
            assert!(part.contains("anna"));
            assert!(part.contains("https://onm.example.org/settings/mail"));
            assert!(!part.contains("smtp-user-geheim") && !part.contains("smtp-pw-geheim"));
        }
        assert!(html.contains("cid:logo@opennetmon"));
        assert!(html.contains("Ausgelöst von <b>anna</b> unter Einstellungen → E-Mail"));
        assert!(mail.text.contains("Ausgelöst von anna unter"));
        assert_eq!(mail.inline.len(), 1);
        assert!(mail.text.contains("Europe/Berlin"));
    }

    #[test]
    fn without_the_address_of_the_web_interface_there_are_no_links() {
        let mail = compose(&config(Lang::En, None), "anna", &clock(Lang::En));
        assert_eq!(mail.subject, "[opennetmon] Test e-mail");
        let html = mail.html.unwrap();
        assert!(!html.contains("href="), "{html}");
        assert!(html.contains("The e-mail settings work"));
        assert!(mail.text.contains("STARTTLS, with login"));
        // A name that looks like markup stays text, also in bold.
        let mail = compose(&config(Lang::En, None), "<i>x</i>", &clock(Lang::En));
        assert!(
            mail.html
                .unwrap()
                .contains("<b>&#60;i&#62;x&#60;/i&#62;</b>")
        );
    }
}
