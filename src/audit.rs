//! The audit log: who changed what and when (administrators' changes, sign-ins including failed
//! ones, sign-outs). Entries are written by the handlers after a change succeeded
//! ([`crate::web::audit::record`]); storage: `storage::audit`, page `/audit`.
//!
//! Secrets never go in: for passwords, community strings and keys the entry only says that they
//! changed ([`Changes::secret`]).

use crate::i18n::{Arg, Msg};
use crate::sensors::Unit;
use crate::status::Thresholds;

/// What was done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Created,
    Changed,
    Deleted,
    SignedIn,
    SignInFailed,
    SignedOut,
}

impl Action {
    pub const ALL: [Self; 6] = [
        Self::Created,
        Self::Changed,
        Self::Deleted,
        Self::SignedIn,
        Self::SignInFailed,
        Self::SignedOut,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Changed => "changed",
            Self::Deleted => "deleted",
            Self::SignedIn => "signed_in",
            Self::SignInFailed => "sign_in_failed",
            Self::SignedOut => "signed_out",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.as_str() == s)
    }

    pub fn label_id(self) -> &'static str {
        match self {
            Self::Created => "audit-action-created",
            Self::Changed => "audit-action-changed",
            Self::Deleted => "audit-action-deleted",
            Self::SignedIn => "audit-action-signed-in",
            Self::SignInFailed => "audit-action-sign-in-failed",
            Self::SignedOut => "audit-action-signed-out",
        }
    }
}

/// What it was done to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Group,
    Device,
    Sensor,
    Profile,
    User,
    Template,
    Mib,
    Mail,
    Schedule,
    Session,
}

impl Kind {
    pub const ALL: [Self; 10] = [
        Self::Group,
        Self::Device,
        Self::Sensor,
        Self::Profile,
        Self::User,
        Self::Template,
        Self::Mib,
        Self::Mail,
        Self::Schedule,
        Self::Session,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Group => "group",
            Self::Device => "device",
            Self::Sensor => "sensor",
            Self::Profile => "profile",
            Self::User => "user",
            Self::Template => "template",
            Self::Mib => "mib",
            Self::Mail => "mail",
            Self::Schedule => "schedule",
            Self::Session => "session",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }

    pub fn label_id(self) -> &'static str {
        match self {
            Self::Group => "audit-kind-group",
            Self::Device => "audit-kind-device",
            Self::Sensor => "audit-kind-sensor",
            Self::Profile => "audit-kind-profile",
            Self::User => "audit-kind-user",
            Self::Template => "audit-kind-template",
            Self::Mib => "audit-kind-mib",
            Self::Mail => "audit-kind-mail",
            Self::Schedule => "audit-kind-schedule",
            Self::Session => "audit-kind-session",
        }
    }

    /// The page of an object of this kind, if it has one.
    pub fn href(self, id: i64) -> Option<String> {
        match self {
            Self::Group => Some(format!("/groups/{id}")),
            Self::Device => Some(format!("/devices/{id}/sensors")),
            Self::Sensor => Some(format!("/sensors/{id}")),
            Self::Profile => Some(format!("/credentials/{id}/edit")),
            Self::User => Some(format!("/users/{id}")),
            Self::Template => Some(format!("/templates/{id}")),
            Self::Schedule => Some(format!("/schedules/{id}")),
            Self::Mib | Self::Mail | Self::Session => None,
        }
    }
}

/// The changed fields of an object: old → new, secrets only as "changed".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Changes(Vec<Msg>);

impl Changes {
    pub fn new() -> Self {
        Self::default()
    }

    /// `field` (a catalog id) changed from `old` to `new`; nothing if they are equal.
    pub fn field(mut self, field: &'static str, old: impl ToString, new: impl ToString) -> Self {
        let (old, new) = (old.to_string(), new.to_string());
        if old != new {
            self.0.push(
                Msg::new("audit-change")
                    .with("field", Msg::new(field))
                    .with("old", shown(old))
                    .with("new", shown(new)),
            );
        }
        self
    }

    /// Like [`Self::field`] for values that are catalog ids (a role, a device type): shown in the
    /// language of the reader.
    pub fn label(mut self, field: &'static str, old: &'static str, new: &'static str) -> Self {
        if old != new {
            self.0.push(
                Msg::new("audit-change")
                    .with("field", Msg::new(field))
                    .with("old", Msg::new(old))
                    .with("new", Msg::new(new)),
            );
        }
        self
    }

    /// Like [`Self::value`] for a value that is a catalog id.
    pub fn value_label(mut self, field: &'static str, value: &'static str) -> Self {
        self.0.push(
            Msg::new("audit-value")
                .with("field", Msg::new(field))
                .with("value", Msg::new(value)),
        );
        self
    }

    /// The changed limits of a channel, with their units.
    pub fn limits(mut self, channel: Msg, unit: Unit, old: &Thresholds, new: &Thresholds) -> Self {
        let pairs = [
            ("threshold-warn-above", old.warn_above, new.warn_above),
            ("threshold-warn-below", old.warn_below, new.warn_below),
            ("threshold-down-above", old.down_above, new.down_above),
            ("threshold-down-below", old.down_below, new.down_below),
        ];
        for (limit, old, new) in pairs {
            if old == new {
                continue;
            }
            let limit_value = |msg: Msg, name: &'static str, value: Option<f64>| match value {
                Some(v) => msg.with_value(name, unit, v),
                None => msg.with(name, "–"),
            };
            let msg = Msg::new("audit-limit")
                .with("channel", channel.clone())
                .with("limit", Msg::new(limit));
            let msg = limit_value(msg, "old", old);
            self.0.push(limit_value(msg, "new", new));
        }
        self
    }

    /// `field` was set to `value` (on creation).
    pub fn value(mut self, field: &'static str, value: impl ToString) -> Self {
        self.0.push(
            Msg::new("audit-value")
                .with("field", Msg::new(field))
                .with("value", shown(value.to_string())),
        );
        self
    }

    /// A secret field: only whether it changed, never the value.
    pub fn secret(mut self, field: &'static str, changed: bool) -> Self {
        if changed {
            self.0
                .push(Msg::new("audit-secret-changed").with("field", Msg::new(field)));
        }
        self
    }

    /// Any other note.
    pub fn note(mut self, msg: Msg) -> Self {
        self.0.push(msg);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn into_msg(self) -> Msg {
        Msg::List(self.0)
    }
}

/// An empty value shows as "–".
fn shown(text: String) -> Arg {
    if text.is_empty() {
        Arg::Text("–".to_owned())
    } else {
        Arg::Text(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Lang;

    #[test]
    fn only_changed_fields_are_listed_and_secrets_never_with_their_value() {
        let msg = Changes::new()
            .field("audit-field-name", "Switch", "Switch Büro")
            .field("audit-field-host", "192.0.2.1", "192.0.2.1")
            .field("audit-field-group", "", "Netzwerk")
            .secret("audit-field-password", true)
            .secret("audit-field-community", false)
            .into_msg();
        let text = Lang::De.msg(&msg);
        assert_eq!(
            text,
            "Name: Switch → Switch Büro, Gruppe: – → Netzwerk, Passwort geändert"
        );
        // Stored and read back.
        assert_eq!(
            Lang::En.msg(&Msg::decode(&msg.encode())),
            "Name: Switch → Switch Büro, Group: – → Netzwerk, Password changed"
        );
        for kind in Kind::ALL {
            assert_eq!(Kind::parse(kind.as_str()), Some(kind));
        }
        for action in Action::ALL {
            assert_eq!(Action::parse(action.as_str()), Some(action));
        }
    }
}
