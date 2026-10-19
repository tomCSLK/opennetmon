//! Persistence of SNMP credential profiles. Secrets are encrypted before they reach the database
//! and only decrypted by [`load_credentials`].

use sqlx::{FromRow, SqlitePool};

use crate::credentials::{
    AuthProtocol, Credentials, PrivProtocol, ProfileInput, ProfileKind, ProfileView, Secret,
    SecurityLevel, SnmpVersion,
};
use crate::crypto::SecretBox;
use crate::error::{Error, Result};
use crate::i18n::Msg;

const CTX_COMMUNITY: &str = "snmp.community";
const CTX_AUTH_PASSWORD: &str = "snmp.auth_password";
const CTX_PRIV_PASSWORD: &str = "snmp.priv_password";

const COLUMNS: &str = "id, name, version, community_enc, username, security_level, auth_protocol, \
                       auth_password_enc, priv_protocol, priv_password_enc";

#[derive(FromRow)]
struct Row {
    id: i64,
    name: String,
    version: String,
    community_enc: Option<Vec<u8>>,
    username: Option<String>,
    security_level: Option<String>,
    auth_protocol: Option<String>,
    auth_password_enc: Option<Vec<u8>>,
    priv_protocol: Option<String>,
    priv_password_enc: Option<Vec<u8>>,
}

fn corrupt(what: &str) -> Error {
    Error::Internal(format!("invalid {what} in credential_profiles"))
}

impl Row {
    fn view(&self) -> Result<ProfileView> {
        Ok(ProfileView {
            id: self.id,
            name: self.name.clone(),
            version: SnmpVersion::parse(&self.version).ok_or_else(|| corrupt("version"))?,
            username: self.username.clone(),
            level: self
                .security_level
                .as_deref()
                .map(|s| SecurityLevel::parse(s).ok_or_else(|| corrupt("security level")))
                .transpose()?,
            auth_protocol: self
                .auth_protocol
                .as_deref()
                .map(|s| AuthProtocol::parse(s).ok_or_else(|| corrupt("auth protocol")))
                .transpose()?,
            priv_protocol: self
                .priv_protocol
                .as_deref()
                .map(|s| PrivProtocol::parse(s).ok_or_else(|| corrupt("priv protocol")))
                .transpose()?,
        })
    }
}

/// The encrypted secrets to store, resolved from the input and the existing row.
struct Resolved {
    community: Option<Vec<u8>>,
    username: Option<String>,
    level: Option<SecurityLevel>,
    auth_protocol: Option<AuthProtocol>,
    auth_password: Option<Vec<u8>>,
    priv_protocol: Option<PrivProtocol>,
    priv_password: Option<Vec<u8>>,
}

/// Uses the submitted secret if there is one, otherwise the stored blob (if any), otherwise fails.
fn secret_or_keep(
    secrets: &SecretBox,
    submitted: Option<&Secret>,
    stored: Option<&Vec<u8>>,
    context: &str,
    missing_message: &'static str,
) -> Result<Vec<u8>> {
    match (submitted, stored) {
        (Some(secret), _) => Ok(secrets.encrypt(secret.expose(), context)?),
        (None, Some(blob)) => Ok(blob.clone()),
        (None, None) => Err(Error::invalid(Msg::new(missing_message))),
    }
}

fn resolve(secrets: &SecretBox, input: &ProfileInput, existing: Option<&Row>) -> Result<Resolved> {
    match &input.kind {
        ProfileKind::V2c { community } => {
            let stored = existing
                .filter(|r| r.version == "v2c")
                .and_then(|r| r.community_enc.as_ref());
            Ok(Resolved {
                community: Some(secret_or_keep(
                    secrets,
                    community.as_ref(),
                    stored,
                    CTX_COMMUNITY,
                    "profile-community-missing",
                )?),
                username: None,
                level: None,
                auth_protocol: None,
                auth_password: None,
                priv_protocol: None,
                priv_password: None,
            })
        }
        ProfileKind::V3 {
            username,
            level,
            auth_protocol,
            auth_password,
            priv_protocol,
            priv_password,
        } => {
            let old = existing.filter(|r| r.version == "v3");
            let auth = secret_or_keep(
                secrets,
                auth_password.as_ref(),
                old.and_then(|r| r.auth_password_enc.as_ref()),
                CTX_AUTH_PASSWORD,
                "profile-auth-password-missing",
            )?;
            let (priv_protocol, priv_password) = match level {
                SecurityLevel::AuthNoPriv => (None, None),
                SecurityLevel::AuthPriv => {
                    let protocol = priv_protocol
                        .ok_or_else(|| Error::invalid(Msg::new("profile-priv-protocol-missing")))?;
                    let password = secret_or_keep(
                        secrets,
                        priv_password.as_ref(),
                        old.and_then(|r| r.priv_password_enc.as_ref()),
                        CTX_PRIV_PASSWORD,
                        "profile-priv-password-missing",
                    )?;
                    (Some(protocol), Some(password))
                }
            };
            Ok(Resolved {
                community: None,
                username: Some(username.clone()),
                level: Some(*level),
                auth_protocol: Some(*auth_protocol),
                auth_password: Some(auth),
                priv_protocol,
                priv_password,
            })
        }
    }
}

/// Creates (`id = None`) or updates a profile and returns its id. Empty secrets keep the stored ones.
pub async fn save_profile(
    pool: &SqlitePool,
    secrets: &SecretBox,
    id: Option<i64>,
    input: &ProfileInput,
) -> Result<i64> {
    input.validate().map_err(Error::invalid)?;

    let existing = match id {
        Some(id) => Some(fetch_row(pool, id).await?.ok_or(Error::NotFound)?),
        None => None,
    };
    let r = resolve(secrets, input, existing.as_ref())?;
    let version = match input.kind {
        ProfileKind::V2c { .. } => SnmpVersion::V2c,
        ProfileKind::V3 { .. } => SnmpVersion::V3,
    };

    let result = match id {
        None => sqlx::query(
            "INSERT INTO credential_profiles (name, version, community_enc, username, security_level, \
             auth_protocol, auth_password_enc, priv_protocol, priv_password_enc) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        ),
        Some(_) => sqlx::query(
            "UPDATE credential_profiles SET name = ?, version = ?, community_enc = ?, username = ?, \
             security_level = ?, auth_protocol = ?, auth_password_enc = ?, priv_protocol = ?, \
             priv_password_enc = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
        ),
    }
    .bind(input.trimmed_name())
    .bind(version.as_str())
    .bind(r.community)
    .bind(r.username)
    .bind(r.level.map(SecurityLevel::as_str))
    .bind(r.auth_protocol.map(AuthProtocol::as_str))
    .bind(r.auth_password)
    .bind(r.priv_protocol.map(PrivProtocol::as_str))
    .bind(r.priv_password);
    let result = match id {
        Some(id) => result.bind(id),
        None => result,
    }
    .execute(pool)
    .await;

    match result {
        Ok(done) => Ok(id.unwrap_or_else(|| done.last_insert_rowid())),
        Err(e)
            if e.as_database_error()
                .is_some_and(|d| d.is_unique_violation()) =>
        {
            Err(Error::conflict(Msg::new("profile-name-taken")))
        }
        Err(e) => Err(e.into()),
    }
}

async fn fetch_row(pool: &SqlitePool, id: i64) -> Result<Option<Row>> {
    Ok(sqlx::query_as::<_, Row>(&format!(
        "SELECT {COLUMNS} FROM credential_profiles WHERE id = ?"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

pub async fn get_profile(pool: &SqlitePool, id: i64) -> Result<Option<ProfileView>> {
    fetch_row(pool, id).await?.map(|r| r.view()).transpose()
}

pub async fn list_profiles(pool: &SqlitePool) -> Result<Vec<ProfileView>> {
    sqlx::query_as::<_, Row>(&format!(
        "SELECT {COLUMNS} FROM credential_profiles ORDER BY name"
    ))
    .fetch_all(pool)
    .await?
    .iter()
    .map(Row::view)
    .collect()
}

/// Returns true if a profile was deleted. A profile that devices still use is refused.
pub async fn delete_profile(pool: &SqlitePool, id: i64) -> Result<bool> {
    let in_use: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM devices WHERE profile_id = ?")
        .bind(id)
        .fetch_one(pool)
        .await?;
    if in_use > 0 {
        return Err(Error::conflict(Msg::new("profile-in-use")));
    }
    let done = sqlx::query("DELETE FROM credential_profiles WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

/// Decrypts a profile for use by the poller.
pub async fn load_credentials(
    pool: &SqlitePool,
    secrets: &SecretBox,
    id: i64,
) -> Result<Option<Credentials>> {
    let Some(row) = fetch_row(pool, id).await? else {
        return Ok(None);
    };
    let need = |blob: &Option<Vec<u8>>, context: &str| -> Result<Secret> {
        let blob = blob.as_ref().ok_or_else(|| corrupt("missing secret"))?;
        Ok(Secret::new(secrets.decrypt(blob, context)?))
    };
    let view = row.view()?;
    Ok(Some(match view.version {
        SnmpVersion::V2c => Credentials::V2c {
            community: need(&row.community_enc, CTX_COMMUNITY)?,
        },
        SnmpVersion::V3 => Credentials::V3 {
            username: view.username.ok_or_else(|| corrupt("username"))?,
            auth_protocol: view.auth_protocol.ok_or_else(|| corrupt("auth protocol"))?,
            auth_password: need(&row.auth_password_enc, CTX_AUTH_PASSWORD)?,
            privacy: match view.priv_protocol {
                Some(protocol) => {
                    Some((protocol, need(&row.priv_password_enc, CTX_PRIV_PASSWORD)?))
                }
                None => None,
            },
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Lang;
    use crate::storage;

    type SecretColumns = (Option<Vec<u8>>, Option<Vec<u8>>, Option<Vec<u8>>);

    async fn setup() -> (SqlitePool, SecretBox, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let pool = storage::connect(&dir.path().join("c.db")).await.unwrap();
        storage::migrate(&pool).await.unwrap();
        (pool, SecretBox::from_key([9u8; 32]), dir)
    }

    fn v2c(name: &str, community: Option<&str>) -> ProfileInput {
        ProfileInput {
            name: name.to_owned(),
            kind: ProfileKind::V2c {
                community: community.map(Secret::new),
            },
        }
    }

    fn v3_no_priv(name: &str, user: &str, auth: Option<&str>) -> ProfileInput {
        ProfileInput {
            name: name.to_owned(),
            kind: ProfileKind::V3 {
                username: user.to_owned(),
                level: SecurityLevel::AuthNoPriv,
                auth_protocol: AuthProtocol::Md5,
                auth_password: auth.map(Secret::new),
                priv_protocol: None,
                priv_password: None,
            },
        }
    }

    fn v3_priv(name: &str, auth: Option<&str>, privp: Option<&str>) -> ProfileInput {
        ProfileInput {
            name: name.to_owned(),
            kind: ProfileKind::V3 {
                username: "monitor".to_owned(),
                level: SecurityLevel::AuthPriv,
                auth_protocol: AuthProtocol::Sha256,
                auth_password: auth.map(Secret::new),
                priv_protocol: Some(PrivProtocol::Aes128),
                priv_password: privp.map(Secret::new),
            },
        }
    }

    #[tokio::test]
    async fn secrets_are_encrypted_at_rest_and_roundtrip() {
        let (pool, sb, _dir) = setup().await;
        let id = save_profile(
            &pool,
            &sb,
            None,
            &v3_priv("Core", Some("auth-secret-1"), Some("priv-secret-1")),
        )
        .await
        .unwrap();
        let v2 = save_profile(&pool, &sb, None, &v2c("Alt", Some("community-xyz")))
            .await
            .unwrap();

        let all: Vec<SecretColumns> = sqlx::query_as(
            "SELECT community_enc, auth_password_enc, priv_password_enc FROM credential_profiles",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        for blob in all.into_iter().flat_map(|(a, b, c)| [a, b, c]).flatten() {
            let text = String::from_utf8_lossy(&blob).into_owned();
            for plain in ["auth-secret-1", "priv-secret-1", "community-xyz"] {
                assert!(!text.contains(plain), "plaintext found in database");
            }
        }

        match load_credentials(&pool, &sb, id).await.unwrap().unwrap() {
            Credentials::V3 {
                username,
                auth_protocol,
                auth_password,
                privacy,
            } => {
                assert_eq!(username, "monitor");
                assert_eq!(auth_protocol, AuthProtocol::Sha256);
                assert_eq!(auth_password.expose(), "auth-secret-1");
                let (p, s) = privacy.unwrap();
                assert_eq!((p, s.expose()), (PrivProtocol::Aes128, "priv-secret-1"));
            }
            other => panic!("unexpected {other:?}"),
        }
        match load_credentials(&pool, &sb, v2).await.unwrap().unwrap() {
            Credentials::V2c { community } => assert_eq!(community.expose(), "community-xyz"),
            other => panic!("unexpected {other:?}"),
        }
        assert!(load_credentials(&pool, &sb, 999).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn omada_style_auth_no_priv_profile() {
        let (pool, sb, _dir) = setup().await;
        let id = save_profile(
            &pool,
            &sb,
            None,
            &v3_no_priv("Omada", "MCS6_SNMP", Some("12345678")),
        )
        .await
        .unwrap();
        let view = get_profile(&pool, id).await.unwrap().unwrap();
        assert_eq!(
            view.summary(Lang::De),
            "SNMPv3 · authNoPriv · MD5 · Benutzer MCS6_SNMP"
        );
        match load_credentials(&pool, &sb, id).await.unwrap().unwrap() {
            Credentials::V3 { privacy, .. } => assert!(privacy.is_none()),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn update_keeps_secrets_when_left_empty_and_replaces_when_given() {
        let (pool, sb, _dir) = setup().await;
        let id = save_profile(
            &pool,
            &sb,
            None,
            &v3_priv("Core", Some("auth-secret-1"), Some("priv-secret-1")),
        )
        .await
        .unwrap();

        // Rename only: secrets stay.
        save_profile(&pool, &sb, Some(id), &v3_priv("Core2", None, None))
            .await
            .unwrap();
        let view = get_profile(&pool, id).await.unwrap().unwrap();
        assert_eq!(view.name, "Core2");
        match load_credentials(&pool, &sb, id).await.unwrap().unwrap() {
            Credentials::V3 {
                auth_password,
                privacy,
                ..
            } => {
                assert_eq!(auth_password.expose(), "auth-secret-1");
                assert_eq!(privacy.unwrap().1.expose(), "priv-secret-1");
            }
            other => panic!("unexpected {other:?}"),
        }

        // Replace only the auth password.
        save_profile(
            &pool,
            &sb,
            Some(id),
            &v3_priv("Core2", Some("new-auth-pass"), None),
        )
        .await
        .unwrap();
        match load_credentials(&pool, &sb, id).await.unwrap().unwrap() {
            Credentials::V3 {
                auth_password,
                privacy,
                ..
            } => {
                assert_eq!(auth_password.expose(), "new-auth-pass");
                assert_eq!(privacy.unwrap().1.expose(), "priv-secret-1");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn downgrading_to_auth_no_priv_drops_the_privacy_secret() {
        let (pool, sb, _dir) = setup().await;
        let id = save_profile(
            &pool,
            &sb,
            None,
            &v3_priv("P", Some("auth-secret-1"), Some("priv-secret-1")),
        )
        .await
        .unwrap();
        save_profile(&pool, &sb, Some(id), &v3_no_priv("P", "monitor", None))
            .await
            .unwrap();
        let leftover: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT priv_password_enc FROM credential_profiles WHERE id = ?")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(leftover.is_none());
    }

    #[tokio::test]
    async fn changing_the_version_requires_new_secrets() {
        let (pool, sb, _dir) = setup().await;
        let id = save_profile(&pool, &sb, None, &v2c("X", Some("public")))
            .await
            .unwrap();
        let err = save_profile(&pool, &sb, Some(id), &v3_no_priv("X", "user", None))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Invalid(_)));
        // The profile is unchanged.
        assert_eq!(
            get_profile(&pool, id).await.unwrap().unwrap().version,
            SnmpVersion::V2c
        );
        save_profile(
            &pool,
            &sb,
            Some(id),
            &v3_no_priv("X", "user", Some("12345678")),
        )
        .await
        .unwrap();
        assert_eq!(
            get_profile(&pool, id).await.unwrap().unwrap().version,
            SnmpVersion::V3
        );
        let community: Option<Vec<u8>> =
            sqlx::query_scalar("SELECT community_enc FROM credential_profiles WHERE id = ?")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(community.is_none());
    }

    #[tokio::test]
    async fn creating_requires_all_secrets() {
        let (pool, sb, _dir) = setup().await;
        assert!(matches!(
            save_profile(&pool, &sb, None, &v2c("A", None)).await,
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            save_profile(&pool, &sb, None, &v3_no_priv("B", "u", None)).await,
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            save_profile(&pool, &sb, None, &v3_priv("C", Some("12345678"), None)).await,
            Err(Error::Invalid(_))
        ));
        assert!(list_profiles(&pool).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn names_are_unique_case_insensitively_and_validated() {
        let (pool, sb, _dir) = setup().await;
        save_profile(&pool, &sb, None, &v2c("Public", Some("public")))
            .await
            .unwrap();
        let err = save_profile(&pool, &sb, None, &v2c("PUBLIC", Some("public")))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Conflict(_)));
        let err = save_profile(&pool, &sb, None, &v2c("  ", Some("public")))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Invalid(_)));
    }

    #[tokio::test]
    async fn list_get_delete_and_missing_ids() {
        let (pool, sb, _dir) = setup().await;
        let b = save_profile(&pool, &sb, None, &v2c("b", Some("x")))
            .await
            .unwrap();
        save_profile(&pool, &sb, None, &v2c("a", Some("x")))
            .await
            .unwrap();
        let names: Vec<String> = list_profiles(&pool)
            .await
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, ["a", "b"]);
        assert!(delete_profile(&pool, b).await.unwrap());
        assert!(!delete_profile(&pool, b).await.unwrap());
        assert!(get_profile(&pool, b).await.unwrap().is_none());
        let err = save_profile(&pool, &sb, Some(b), &v2c("b", Some("x")))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::NotFound));
    }

    #[tokio::test]
    async fn a_ciphertext_moved_to_another_column_does_not_decrypt() {
        let (pool, sb, _dir) = setup().await;
        let id = save_profile(
            &pool,
            &sb,
            None,
            &v3_priv("Core", Some("auth-secret-1"), Some("priv-secret-1")),
        )
        .await
        .unwrap();
        sqlx::query(
            "UPDATE credential_profiles SET priv_password_enc = auth_password_enc WHERE id = ?",
        )
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
        assert!(load_credentials(&pool, &sb, id).await.is_err());
    }

    #[tokio::test]
    async fn database_rejects_inconsistent_rows() {
        let (pool, _sb, _dir) = setup().await;
        let bad = sqlx::query(
            "INSERT INTO credential_profiles (name, version, username) VALUES ('bad', 'v3', 'u')",
        )
        .execute(&pool)
        .await;
        assert!(bad.is_err());
    }
}
