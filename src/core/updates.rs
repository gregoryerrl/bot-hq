//! Check GitHub Releases for a newer bot-hq and report it to the UI.
//!
//! This is the "check-and-notify" half of the update story (scope A): we poll
//! the GitHub releases API, compare the latest tag to the running version, and
//! hand a [`UpdateInfo`] to the frontend, which shows a "download" banner. No
//! code-signing / updater plugin is involved — the user installs the new build
//! themselves. A future full auto-install (Tauri updater plugin) reuses this
//! same [`UpdateInfo`] + the banner shell.
//!
//! The pure decision logic (version compare, HTTP-status → release mapping,
//! `UpdateInfo` construction) is split from the thin async fetch so it is
//! unit-testable without touching the network. The Tauri command in
//! `tauri_cmd::updates` wraps [`check_for_update`].

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use specta::Type;

/// GitHub "latest release" API for this repo. `/releases/latest` excludes
/// drafts + pre-releases by design — exactly the user-facing-stable set we want.
pub const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/gregoryerrl/bot-hq/releases/latest";

/// The subset of the GitHub release JSON we care about. Unknown fields are
/// ignored by serde, so the full (large) payload deserializes fine.
#[derive(Debug, Clone, Deserialize)]
pub struct GithubRelease {
    pub tag_name: String,
    pub html_url: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub published_at: Option<String>,
}

/// Update status reported to the frontend. snake_case fields (mirrors the
/// `SessionInfo` return-type convention) — the React side reads these names.
#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub release_url: String,
    pub release_notes: Option<String>,
    pub published_at: Option<String>,
}

/// Strip a leading `v`/`V` so `v0.1.1` and `0.1.1` parse the same.
fn normalize(tag: &str) -> &str {
    tag.strip_prefix('v')
        .or_else(|| tag.strip_prefix('V'))
        .unwrap_or(tag)
}

/// True iff `latest_tag` parses as a strictly-higher semver than `current`.
/// Leading `v` is stripped on both sides; unparseable input on either side
/// returns false (never nag the user over a malformed tag).
pub fn is_newer(current: &str, latest_tag: &str) -> bool {
    match (
        semver::Version::parse(normalize(current)),
        semver::Version::parse(normalize(latest_tag)),
    ) {
        (Ok(cur), Ok(latest)) => latest > cur,
        _ => false,
    }
}

/// Map an HTTP status + body from the releases endpoint to an optional release.
/// - `404` → `Ok(None)`: no releases cut yet (the current real-world state) —
///   this is NOT an error.
/// - `2xx` → parse the body (`Err` on malformed JSON).
/// - anything else (403 rate-limit, 5xx, …) → `Err`.
pub fn release_from_response(status: u16, body: &str) -> Result<Option<GithubRelease>> {
    match status {
        404 => Ok(None),
        200..=299 => {
            let rel: GithubRelease = serde_json::from_str(body)
                .map_err(|e| anyhow!("could not parse GitHub release JSON: {e}"))?;
            Ok(Some(rel))
        }
        other => Err(anyhow!("GitHub releases API returned HTTP {other}")),
    }
}

/// Build the UI-facing status. `None` (no release found) → not-available with
/// `latest_version == current`.
pub fn build_update_info(release: Option<&GithubRelease>, current: &str) -> UpdateInfo {
    match release {
        Some(rel) => UpdateInfo {
            current_version: current.to_string(),
            latest_version: normalize(&rel.tag_name).to_string(),
            update_available: is_newer(current, &rel.tag_name),
            release_url: rel.html_url.clone(),
            release_notes: rel.body.clone(),
            published_at: rel.published_at.clone(),
        },
        None => UpdateInfo {
            current_version: current.to_string(),
            latest_version: current.to_string(),
            update_available: false,
            release_url: String::new(),
            release_notes: None,
            published_at: None,
        },
    }
}

/// GET the latest release. Thin glue over [`release_from_response`], where the
/// testable logic lives. A `User-Agent` header is mandatory — GitHub 403s
/// requests without one.
pub async fn fetch_latest_release(
    client: &reqwest::Client,
    api_url: &str,
) -> Result<Option<GithubRelease>> {
    let resp = client
        .get(api_url)
        .header(
            reqwest::header::USER_AGENT,
            concat!("bot-hq/", env!("CARGO_PKG_VERSION")),
        )
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| anyhow!("request to GitHub failed: {e}"))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    release_from_response(status, &body)
}

/// Full check: fetch the latest release then build the UI status. `current` is
/// normally `env!("CARGO_PKG_VERSION")` from the calling crate.
pub async fn check_for_update(
    client: &reqwest::Client,
    api_url: &str,
    current: &str,
) -> Result<UpdateInfo> {
    let release = fetch_latest_release(client, api_url).await?;
    Ok(build_update_info(release.as_ref(), current))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_release_json(tag: &str) -> String {
        format!(
            r#"{{"tag_name":"{tag}","html_url":"https://github.com/gregoryerrl/bot-hq/releases/tag/{tag}","name":"{tag}","body":"Fixes - a thing","published_at":"2026-06-10T00:00:00Z","draft":false,"prerelease":false}}"#
        )
    }

    #[test]
    fn is_newer_detects_higher_minor() {
        assert!(is_newer("0.1.0", "0.2.0"));
    }

    #[test]
    fn is_newer_ignores_leading_v() {
        assert!(is_newer("0.1.0", "v0.1.1"));
    }

    #[test]
    fn is_newer_compares_numerically_not_lexically() {
        // 0.10.0 > 0.9.0 numerically (a lexical string compare gets this wrong).
        assert!(is_newer("0.9.0", "0.10.0"));
    }

    #[test]
    fn is_newer_false_when_equal() {
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("1.2.3", "v1.2.3"));
    }

    #[test]
    fn is_newer_false_when_older() {
        assert!(!is_newer("2.0.0", "1.9.9"));
    }

    #[test]
    fn is_newer_false_on_garbage() {
        assert!(!is_newer("0.1.0", "not-a-version"));
        assert!(!is_newer("garbage", "0.2.0"));
    }

    #[test]
    fn release_from_response_404_is_no_release_not_error() {
        let got = release_from_response(404, "").expect("404 must be Ok(None)");
        assert!(got.is_none());
    }

    #[test]
    fn release_from_response_200_parses_release() {
        let json = sample_release_json("v0.2.0");
        let rel = release_from_response(200, &json)
            .expect("200 valid must parse")
            .expect("200 valid must be Some");
        assert_eq!(rel.tag_name, "v0.2.0");
        assert_eq!(
            rel.html_url,
            "https://github.com/gregoryerrl/bot-hq/releases/tag/v0.2.0"
        );
        assert!(rel.body.unwrap().contains("Fixes"));
        assert_eq!(rel.published_at.as_deref(), Some("2026-06-10T00:00:00Z"));
    }

    #[test]
    fn release_from_response_rate_limit_is_error() {
        assert!(release_from_response(403, "rate limited").is_err());
    }

    #[test]
    fn release_from_response_malformed_json_is_error() {
        assert!(release_from_response(200, "{not json").is_err());
    }

    #[test]
    fn build_update_info_flags_available_when_newer() {
        let rel = release_from_response(200, &sample_release_json("v0.2.0"))
            .unwrap()
            .unwrap();
        let info = build_update_info(Some(&rel), "0.1.0");
        assert!(info.update_available);
        assert_eq!(info.current_version, "0.1.0");
        assert_eq!(info.latest_version, "0.2.0"); // normalized — no leading v
        assert_eq!(
            info.release_url,
            "https://github.com/gregoryerrl/bot-hq/releases/tag/v0.2.0"
        );
        assert!(info.release_notes.is_some());
    }

    #[test]
    fn build_update_info_not_available_when_same() {
        let rel = release_from_response(200, &sample_release_json("v0.1.0"))
            .unwrap()
            .unwrap();
        let info = build_update_info(Some(&rel), "0.1.0");
        assert!(!info.update_available);
        assert_eq!(info.latest_version, "0.1.0");
    }

    #[test]
    fn build_update_info_none_means_no_update() {
        let info = build_update_info(None, "0.1.0");
        assert!(!info.update_available);
        assert_eq!(info.latest_version, "0.1.0");
        assert!(info.release_url.is_empty());
    }
}
