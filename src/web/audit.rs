//! The audit log page `/audit` (administrators only) and [`record`], which the handlers call after a
//! change succeeded. See [`crate::audit`].

use askama::Template;
use axum::extract::{Query, State};
use axum::response::Html;
use serde::Deserialize;

use crate::audit::{Action, Changes, Kind};
use crate::error::Result;
use crate::i18n::{Lang, Msg};
use crate::pause::Target;
use crate::storage::audit::{self as store, NewEntry};
use crate::web::auth::CurrentUser;
use crate::web::sensors::now_ms;
use crate::web::{AppState, Layout, time_html};

/// Entries shown on the page (the newest).
const SHOWN: u32 = 2_000;

/// What happened to which object, for [`record`].
pub struct Object<'a> {
    pub kind: Kind,
    pub id: Option<i64>,
    pub name: &'a str,
}

impl<'a> Object<'a> {
    pub fn new(kind: Kind, id: i64, name: &'a str) -> Self {
        Self {
            kind,
            id: Some(id),
            name,
        }
    }
}

impl<'a> Object<'a> {
    /// A group, device or sensor as the pause and dependency pages name it.
    pub fn target(target: Target, name: &'a str) -> Self {
        let (kind, id) = match target {
            Target::Group(id) => (Kind::Group, id),
            Target::Device(id) => (Kind::Device, id),
            Target::Sensor(id) => (Kind::Sensor, id),
        };
        Self::new(kind, id, name)
    }
}

/// Writes an entry for a change by `user`; a change without differences (a form saved unchanged)
/// writes none. A failure is logged and does not fail the request: the change itself has been
/// made already.
pub async fn record(
    state: &AppState,
    user: &CurrentUser,
    action: Action,
    object: Object<'_>,
    changes: Changes,
) {
    if action == Action::Changed && changes.is_empty() {
        return;
    }
    write(
        state,
        &user.username,
        user.address.map(|a| a.to_string()),
        action,
        object,
        &changes.into_msg(),
    )
    .await;
}

/// Like [`record`] without a signed-in user (sign-in attempts).
pub async fn write(
    state: &AppState,
    user: &str,
    address: Option<String>,
    action: Action,
    object: Object<'_>,
    detail: &Msg,
) {
    let entry = NewEntry {
        ts_ms: now_ms(),
        user,
        address,
        action,
        kind: object.kind,
        object_id: object.id,
        object_name: object.name,
        detail,
    };
    if let Err(e) = store::insert(&state.pool, &entry).await {
        tracing::error!(error = %e, "writing the audit log failed");
    }
}

struct Row {
    at: String,
    user: String,
    address: String,
    action: String,
    /// Badge class: changes neutral, deletions and failed sign-ins red.
    level: &'static str,
    kind: String,
    name: String,
    href: Option<String>,
    detail: String,
    filter: String,
}

#[derive(Template)]
#[template(path = "audit.html")]
struct AuditTemplate {
    t: Lang,
    layout: Layout,
    rows: Vec<Row>,
    /// "Benutzer anna" when filtered.
    filtered: String,
}

#[derive(Deserialize, Default)]
pub struct AuditQuery {
    user: Option<String>,
}

pub async fn page(
    State(state): State<AppState>,
    user: CurrentUser,
    Query(query): Query<AuditQuery>,
) -> Result<Html<String>> {
    user.require_admin()?;
    let t = user.lang;
    let mut entries = store::list(&state.pool, SHOWN).await?;
    if let Some(name) = query.user.as_deref().filter(|n| !n.is_empty()) {
        entries.retain(|e| e.user == name);
    }
    let rows = entries
        .into_iter()
        .map(|e| {
            let action = t.get(e.action.label_id());
            let mut kind = t.get(e.kind.label_id());
            // Objects without a name of their own (the mail settings) show their kind instead.
            let name = if e.object_name.is_empty() {
                std::mem::take(&mut kind)
            } else {
                e.object_name
            };
            let detail = t.msg(&e.detail);
            let address = e.address.unwrap_or_default();
            let filter = [e.user.as_str(), &address, &action, &kind, &name, &detail]
                .join(" ")
                .to_lowercase();
            Row {
                at: time_html(e.ts_ms),
                level: match e.action {
                    Action::Deleted | Action::SignInFailed => "status-down",
                    Action::Created => "status-up",
                    _ => "status-paused",
                },
                href: e.object_id.and_then(|id| e.kind.href(id)),
                user: e.user,
                address,
                action,
                kind,
                name,
                detail,
                filter,
            }
        })
        .collect();
    let page = AuditTemplate {
        layout: user.layout(),
        rows,
        filtered: query
            .user
            .filter(|n| !n.is_empty())
            .map(|n| t.get1("audit-filtered-user", "user", n.as_str()))
            .unwrap_or_default(),
        t,
    };
    Ok(Html(page.render()?))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::*;
    use crate::web::testutil::*;

    const COMMUNITY: &str = "community-sehr-geheim";
    const NEW_COMMUNITY: &str = "community-noch-geheimer";
    const SMTP_PW: &str = "smtp-passwort-geheim";
    const USER_PW: &str = "ein-neues-langes-passwort";
    const TYPED_IN_NAME: &str = "passwort-im-benutzerfeld";

    async fn entries(h: &Harness) -> Vec<store::Entry> {
        store::list(&h.state.pool, 1_000).await.unwrap()
    }

    fn has(entries: &[store::Entry], action: Action, kind: Kind, name: &str) -> bool {
        entries
            .iter()
            .any(|e| e.action == action && e.kind == kind && e.object_name == name)
    }

    #[tokio::test]
    async fn changes_sign_ins_and_sign_outs_are_logged_without_any_secret() {
        let h = harness(true).await;
        let (admin, csrf) = login(&h, "anna").await;
        let ok = |status: StatusCode| assert_eq!(status, StatusCode::SEE_OTHER);

        // Groups
        let form = format!("csrf_token={csrf}&name=Netzwerk&parent_id=");
        ok(send(&h.app, post("/groups", Some(&admin), &form))
            .await
            .status);
        let form = format!("csrf_token={csrf}&name=Netz&parent_id=");
        ok(send(&h.app, post("/groups/1", Some(&admin), &form))
            .await
            .status);
        // Saved unchanged: no entry.
        ok(send(&h.app, post("/groups/1", Some(&admin), &form))
            .await
            .status);

        // SNMP profile with a community, changed with a new one
        let form = format!(
            "csrf_token={csrf}&name=Switche&version=v2c&community={COMMUNITY}&username=&level=auth_no_priv\
             &auth_protocol=md5&auth_password=&priv_protocol=&priv_password="
        );
        ok(send(&h.app, post("/credentials", Some(&admin), &form))
            .await
            .status);
        let form = form.replace(COMMUNITY, NEW_COMMUNITY);
        ok(send(&h.app, post("/credentials/1", Some(&admin), &form))
            .await
            .status);

        // Device, changed, a sensor on it renamed, then deleted
        let form = format!(
            "csrf_token={csrf}&name=Switch&host=192.0.2.10&group_id=1&profile_id=1\
             &port=161&interval_secs=60&timeout_secs=5&retries=1"
        );
        ok(send(&h.app, post("/devices", Some(&admin), &form))
            .await
            .status);
        let form = form.replace("name=Switch", "name=Switch+B%C3%BCro");
        ok(send(&h.app, post("/devices/1", Some(&admin), &form))
            .await
            .status);
        let sensor = crate::storage::sensors::create_sensor(
            &h.state.pool,
            1,
            "Uptime",
            &crate::sensors::spec::SensorSpec::Uptime,
        )
        .await
        .unwrap();
        let form = format!("csrf_token={csrf}&name=Laufzeit");
        let reply = send(
            &h.app,
            post(&format!("/sensors/{sensor}/rename"), Some(&admin), &form),
        )
        .await;
        ok(reply.status);
        let form = format!("csrf_token={csrf}&confirm=yes");
        ok(send(&h.app, post("/devices/1/delete", Some(&admin), &form))
            .await
            .status);

        // A user's password and the mail settings with an SMTP password
        let form = format!("csrf_token={csrf}&password={USER_PW}&password_repeat={USER_PW}");
        ok(send(&h.app, post("/users/2/password", Some(&admin), &form))
            .await
            .status);
        let form = format!(
            "csrf_token={csrf}&enabled=on&host=mail.example.org&port=587&security=starttls\
             &username=opennetmon&password={SMTP_PW}&from_address=opennetmon%40example.org\
             &recipients=admin%40example.org&language=de"
        );
        ok(send(&h.app, post("/settings/mail", Some(&admin), &form))
            .await
            .status);

        // Failed sign-ins: a known user with a wrong password, and a password typed as user name
        for name in ["anna", TYPED_IN_NAME] {
            let (cookie, login_csrf) = open_login(&h).await;
            let form = format!("username={name}&password=falsch-falsch&csrf_token={login_csrf}");
            let reply = send(&h.app, post("/login", Some(&cookie), &form)).await;
            assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
        }

        let all = entries(&h).await;
        assert!(has(&all, Action::Created, Kind::Group, "Netzwerk"));
        assert!(has(&all, Action::Changed, Kind::Group, "Netz"));
        assert_eq!(
            all.iter().filter(|e| e.kind == Kind::Group).count(),
            2,
            "a form saved unchanged writes nothing"
        );
        assert!(has(&all, Action::Created, Kind::Profile, "Switche"));
        assert!(has(&all, Action::Changed, Kind::Profile, "Switche"));
        assert!(has(&all, Action::Created, Kind::Device, "Switch"));
        assert!(has(&all, Action::Changed, Kind::Device, "Switch Büro"));
        assert!(has(&all, Action::Changed, Kind::Sensor, "Laufzeit"));
        assert!(has(&all, Action::Deleted, Kind::Device, "Switch Büro"));
        assert!(has(&all, Action::Changed, Kind::User, "bert"));
        assert!(has(&all, Action::Changed, Kind::Mail, ""));
        assert!(has(&all, Action::SignedIn, Kind::Session, ""));
        let failed: Vec<_> = all
            .iter()
            .filter(|e| e.action == Action::SignInFailed)
            .map(|e| (e.user.as_str(), Lang::De.msg(&e.detail)))
            .collect();
        assert_eq!(
            failed,
            [
                ("", "unbekannter Benutzername".to_owned()),
                ("anna", "falsches Passwort".to_owned())
            ]
        );
        let changed = |kind: Kind, name: &str| {
            let e = all
                .iter()
                .find(|e| e.action == Action::Changed && e.kind == kind && e.object_name == name)
                .unwrap();
            Lang::De.msg(&e.detail)
        };
        assert_eq!(changed(Kind::Group, "Netz"), "Name: Netzwerk → Netz");
        assert_eq!(
            changed(Kind::Device, "Switch Büro"),
            "Name: Switch → Switch Büro"
        );
        assert_eq!(changed(Kind::Profile, "Switche"), "Community geändert");
        assert_eq!(changed(Kind::Sensor, "Laufzeit"), "Name: Uptime → Laufzeit");
        assert_eq!(changed(Kind::User, "bert"), "Passwort geändert");
        assert!(changed(Kind::Mail, "").contains("SMTP-Server: – → mail.example.org"));
        assert!(changed(Kind::Mail, "").contains("Passwort geändert"));

        // Signing out is logged as well.
        let form = format!("csrf_token={csrf}");
        ok(send(&h.app, post("/logout", Some(&admin), &form))
            .await
            .status);
        let all = entries(&h).await;
        assert_eq!(all[0].action, Action::SignedOut);
        assert_eq!(all[0].user, "anna");

        // No secret anywhere: not in the stored entries, not on the page in either language.
        let (admin, _) = login(&h, "anna").await;
        let de = send(&h.app, get("/audit", Some(&admin))).await;
        let en = send(&h.app, accepting(get("/audit", Some(&admin)), "en")).await;
        assert_eq!(de.status, StatusCode::OK);
        assert!(de.body.contains("Community geändert"));
        let stored: String = entries(&h)
            .await
            .iter()
            .map(|e| {
                format!(
                    "{} {} {:?} {}",
                    e.user,
                    e.object_name,
                    e.address,
                    e.detail.encode()
                )
            })
            .collect();
        for secret in [
            COMMUNITY,
            NEW_COMMUNITY,
            SMTP_PW,
            USER_PW,
            TYPED_IN_NAME,
            PASSWORD,
        ] {
            for (what, text) in [
                ("entries", &stored),
                ("page", &de.body),
                ("page en", &en.body),
            ] {
                assert!(!text.contains(secret), "{secret} in the {what}");
            }
        }
    }

    #[tokio::test]
    async fn only_admins_see_the_audit_log_and_it_filters_by_user() {
        let h = harness(true).await;
        let (viewer, _) = login(&h, "bert").await;
        assert_eq!(
            send(&h.app, get("/audit", Some(&viewer))).await.status,
            StatusCode::FORBIDDEN
        );
        let (admin, _) = login(&h, "anna").await;
        let page = send(&h.app, get("/audit", Some(&admin))).await;
        assert_eq!(page.status, StatusCode::OK);
        assert!(page.body.contains("href=\"/audit?user=bert\""));
        assert!(page.body.contains("href=\"/audit?user=anna\""));
        let filtered = send(&h.app, get("/audit?user=bert", Some(&admin))).await;
        assert!(filtered.body.contains("Benutzer bert"));
        assert!(!filtered.body.contains("href=\"/audit?user=anna\""));
    }

    #[tokio::test]
    async fn old_entries_are_removed_by_the_retention() {
        let h = harness(true).await;
        let detail = Msg::List(Vec::new());
        for ts_ms in [1_000, 5_000] {
            let entry = NewEntry {
                ts_ms,
                user: "anna",
                address: None,
                action: Action::SignedIn,
                kind: Kind::Session,
                object_id: None,
                object_name: "",
                detail: &detail,
            };
            store::insert(&h.state.pool, &entry).await.unwrap();
        }
        assert_eq!(
            store::apply_retention(&h.state.pool, 2_000).await.unwrap(),
            1
        );
        let left = entries(&h).await;
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].ts_ms, 5_000);
    }
}
