//! Looks once a day whether a newer release exists on GitHub (config `update_check`) and keeps the answer
//! in memory; administrators see it in the sidebar. Nothing is installed: updating stays `apt install`.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::http_client::{self, Options};

/// First check after the start (not right away, so restarts do not hammer the API).
const FIRST_CHECK: Duration = Duration::from_secs(5 * 60);
const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// The answer of the API is a few kilobytes (it includes the release notes).
const MAX_ANSWER: usize = 512 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// e.g. `0.6.0`
    pub version: String,
    /// The release page.
    pub url: String,
}

/// The newer release found by the last check (shared between the check and the web server).
#[derive(Debug, Clone, Default)]
pub struct Latest(Arc<RwLock<Option<Release>>>);

impl Latest {
    pub fn get(&self) -> Option<Release> {
        self.0.read().ok().and_then(|r| r.clone())
    }

    pub fn set(&self, release: Option<Release>) {
        if let Ok(mut slot) = self.0.write() {
            *slot = release;
        }
    }
}

/// `1.2.3` or `v1.2.3` (a suffix like `-rc1` is not a release version).
fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.strip_prefix('v').unwrap_or(text).split('.');
    let mut next = || parts.next()?.parse::<u64>().ok();
    let version = (next()?, next()?, next()?);
    parts.next().is_none().then_some(version)
}

/// Whether `tag` names a version newer than `current`.
pub fn is_newer(current: &str, tag: &str) -> bool {
    match (parse_version(current), parse_version(tag)) {
        (Some(current), Some(tag)) => tag > current,
        _ => false,
    }
}

/// The address of the API for the latest release of `repository` (a `https://github.com/<owner>/<name>` URL).
pub fn api_url(repository: &str) -> Option<String> {
    let path = repository
        .strip_prefix("https://github.com/")?
        .trim_end_matches('/');
    let (owner, name) = path.split_once('/')?;
    let fine = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    };
    (fine(owner) && fine(name))
        .then(|| format!("https://api.github.com/repos/{owner}/{name}/releases/latest"))
}

/// The newer release in an answer of the API, if the answer names one. Drafts and pre-releases do not count,
/// and the link must point to the releases of `repository`.
pub fn newer_release(answer: &[u8], current: &str, repository: &str) -> Option<Release> {
    let value: serde_json::Value = serde_json::from_slice(answer).ok()?;
    let flag = |key: &str| value.get(key).and_then(serde_json::Value::as_bool) == Some(true);
    if flag("draft") || flag("prerelease") {
        return None;
    }
    let tag = value.get("tag_name")?.as_str()?;
    let url = value.get("html_url")?.as_str()?;
    let releases = format!("{}/releases/", repository.trim_end_matches('/'));
    if !url.starts_with(&releases) || url.chars().any(|c| c.is_control() || c == '"') {
        return None;
    }
    is_newer(current, tag).then(|| Release {
        version: tag.strip_prefix('v').unwrap_or(tag).to_owned(),
        url: url.to_owned(),
    })
}

/// Asks `api` once; `Ok(None)` = this version is the newest.
pub async fn check(api: &str, current: &str, repository: &str) -> Result<Option<Release>, String> {
    let options = Options {
        get: true,
        follow_redirects: true,
        verify_tls: true,
        max_body: MAX_ANSWER,
        timeout: Duration::from_secs(20),
    };
    let response = http_client::fetch(api, &options)
        .await
        .map_err(|e| e.to_string())?;
    if !response.status.is_success() {
        return Err(format!("HTTP {}", response.status.as_u16()));
    }
    Ok(newer_release(&response.body, current, repository))
}

/// Checks in the background: a few minutes after the start, then once a day.
pub fn spawn(latest: Latest, current: &'static str, repository: &'static str) {
    let Some(api) = api_url(repository) else {
        tracing::warn!(
            repository,
            "update check: not a GitHub repository, not checking"
        );
        return;
    };
    tokio::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            match check(&api, current, repository).await {
                Ok(found) => {
                    if let Some(release) = &found {
                        tracing::info!(version = %release.version, "a newer version is available");
                    } else {
                        tracing::debug!("update check: this is the newest version");
                    }
                    latest.set(found);
                }
                // No internet, rate limit …: try again tomorrow, never bother the user.
                Err(e) => tracing::debug!(error = %e, "update check failed"),
            }
            tokio::time::sleep(INTERVAL).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http_client::testserver::{ok, serve, status};

    const REPO: &str = "https://github.com/example/opennetmon";

    fn answer(tag: &str, prerelease: bool) -> String {
        format!(
            r#"{{"tag_name":"{tag}","html_url":"{REPO}/releases/tag/{tag}","draft":false,"prerelease":{prerelease},"body":"notes"}}"#
        )
    }

    #[test]
    fn versions_compare_by_number() {
        assert!(is_newer("0.5.0", "v0.6.0"));
        assert!(is_newer("0.5.9", "0.5.10"));
        assert!(is_newer("0.9.0", "v1.0.0"));
        assert!(!is_newer("0.5.0", "v0.5.0"));
        assert!(!is_newer("0.6.0", "v0.5.9"));
        for odd in ["v0.6", "v0.6.0-rc1", "latest", "", "v0.6.0.1"] {
            assert!(!is_newer("0.5.0", odd), "{odd}");
        }
    }

    #[test]
    fn the_api_address_comes_from_the_repository() {
        assert_eq!(
            api_url("https://github.com/tomCSLK/opennetmon").as_deref(),
            Some("https://api.github.com/repos/tomCSLK/opennetmon/releases/latest")
        );
        assert_eq!(api_url("https://gitlab.com/a/b"), None);
        assert_eq!(api_url("https://github.com/a"), None);
        assert_eq!(api_url("https://github.com/a/b?x"), None);
    }

    #[test]
    fn only_newer_regular_releases_of_this_repository_count() {
        let found = newer_release(answer("v0.6.0", false).as_bytes(), "0.5.0", REPO).unwrap();
        assert_eq!(found.version, "0.6.0");
        assert_eq!(found.url, format!("{REPO}/releases/tag/v0.6.0"));
        assert_eq!(
            newer_release(answer("v0.6.0", true).as_bytes(), "0.5.0", REPO),
            None
        );
        assert_eq!(
            newer_release(answer("v0.5.0", false).as_bytes(), "0.5.0", REPO),
            None
        );
        let elsewhere = answer("v0.6.0", false).replace(REPO, "https://evil.example/x");
        assert_eq!(newer_release(elsewhere.as_bytes(), "0.5.0", REPO), None);
        assert_eq!(newer_release(b"not json", "0.5.0", REPO), None);
    }

    #[tokio::test]
    async fn asks_the_api_and_reports_failures() {
        let addr = serve(vec![ok(&answer("v9.0.0", false)), status(403, "")], None).await;
        let api = format!("http://{addr}/repos/example/opennetmon/releases/latest");
        let found = check(&api, "0.5.0", REPO).await.unwrap();
        assert_eq!(found.map(|r| r.version).as_deref(), Some("9.0.0"));
        assert_eq!(check(&api, "0.5.0", REPO).await, Err("HTTP 403".to_owned()));
    }
}
