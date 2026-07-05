//! Plugin manifest parsing.
//!
//! Manifests are JSON files at `<data_dir>/plugins/<plugin_id>/manifest.json`.
//! Schema (v1):
//!
//! ```json
//! {
//!   "id": "discord",
//!   "name": "Discord Bridge",
//!   "version": "0.1.0",
//!   "entry": "index.html",
//!   "requested_capabilities": ["cl_index_search", "session_doc_search"],
//!   "slots": [
//!     { "slot_name": "sidebar.bottom", "panel_route": null },
//!     { "slot_name": null, "panel_route": "/plugins/discord" }
//!   ]
//! }
//! ```
//!
//! The `id` field doubles as the URL host (`plugin-<id>.localhost`) so it
//! must be a valid hostname segment: lowercase ASCII, digits, and `-`.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub entry: String,
    /// Plugin-API contract version (the grantable-command catalog +
    /// postMessage RPC shape). This binary supports exactly 1; parsing
    /// rejects anything else so an old bot-hq can't half-run a newer
    /// plugin. Omitted in JSON = 1.
    #[serde(default = "default_api_version")]
    pub api_version: u32,
    #[serde(default)]
    pub requested_capabilities: Vec<String>,
    #[serde(default)]
    pub slots: Vec<PluginSlot>,
    /// Optional extra CSP origins, appended (never replacing) to the default
    /// source lists of four directives — see docs/PLUGINS.md. This field is
    /// what the user APPROVES at install; serving reads the grant frozen
    /// into `plugins.csp_json` at consent time, never this field directly.
    /// Content rules are enforced by [`validate_csp_extra_origins`] at
    /// preview/install only — parse stays tolerant so manifests stored by
    /// older hosts keep loading (their grant column is NULL → default CSP).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub csp_extra_origins: Option<CspExtraOrigins>,
}

fn default_api_version() -> u32 {
    1
}

/// Extra origins a plugin may request per CSP directive (v1: exactly these
/// four directives; explicit `https://host[:port]` origins only). Unknown
/// keys inside the JSON object are IGNORED at parse (so old installs stay
/// loadable + uninstallable after a host upgrade) and REJECTED at
/// preview/install by [`validate_csp_extra_origins`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type, PartialEq)]
pub struct CspExtraOrigins {
    #[serde(rename = "script-src", default, skip_serializing_if = "Vec::is_empty")]
    pub script_src: Vec<String>,
    #[serde(rename = "style-src", default, skip_serializing_if = "Vec::is_empty")]
    pub style_src: Vec<String>,
    #[serde(rename = "font-src", default, skip_serializing_if = "Vec::is_empty")]
    pub font_src: Vec<String>,
    #[serde(rename = "img-src", default, skip_serializing_if = "Vec::is_empty")]
    pub img_src: Vec<String>,
}

impl CspExtraOrigins {
    /// True when no directive lists any origin — semantically identical to
    /// the field being absent (the default CSP applies unchanged).
    pub fn is_empty(&self) -> bool {
        self.script_src.is_empty()
            && self.style_src.is_empty()
            && self.font_src.is_empty()
            && self.img_src.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq)]
pub struct PluginSlot {
    /// React shell slot name (e.g., "sidebar.bottom"). `None` means the
    /// plugin contributes a dedicated panel via [`panel_route`] instead.
    pub slot_name: Option<String>,
    /// Frontend router path for a dedicated panel. `None` means slot-only.
    pub panel_route: Option<String>,
}

impl PluginManifest {
    /// Parse + validate a manifest JSON blob.
    pub fn parse(json: &str) -> Result<Self> {
        let m: Self = serde_json::from_str(json)
            .map_err(|e| anyhow!("invalid manifest json: {e}"))?;
        m.validate()?;
        Ok(m)
    }

    fn validate(&self) -> Result<()> {
        if self.api_version != 1 {
            return Err(anyhow!(
                "unsupported api_version {} (this bot-hq supports 1)",
                self.api_version
            ));
        }
        if self.id.is_empty() {
            return Err(anyhow!("manifest.id must not be empty"));
        }
        if !is_valid_id(&self.id) {
            return Err(anyhow!(
                "manifest.id must be lowercase alphanumeric or '-' (got: {})",
                self.id
            ));
        }
        if self.name.is_empty() {
            return Err(anyhow!("manifest.name must not be empty"));
        }
        if self.version.is_empty() {
            return Err(anyhow!("manifest.version must not be empty"));
        }
        if self.entry.is_empty() {
            return Err(anyhow!("manifest.entry must not be empty"));
        }
        for cap in &self.requested_capabilities {
            if cap.is_empty() || cap.contains(' ') {
                return Err(anyhow!(
                    "requested_capabilities entries must be tokens (got: {cap:?})"
                ));
            }
        }
        for slot in &self.slots {
            if slot.slot_name.is_none() && slot.panel_route.is_none() {
                return Err(anyhow!(
                    "each slot must specify slot_name OR panel_route"
                ));
            }
        }
        Ok(())
    }

    /// The expected iframe origin for this plugin.
    pub fn iframe_origin(&self) -> String {
        format!("https://plugin-{}.localhost", self.id)
    }
}

fn is_valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

/// The only directives `csp_extra_origins` may extend (v1). Never
/// default-src / connect-src / frame-src — widening those is a different
/// security tier.
pub const CSP_EXTRA_DIRECTIVES: [&str; 4] =
    ["script-src", "style-src", "font-src", "img-src"];

/// Sanity bound on header size; a manifest wanting more origins than this
/// per directive is doing something the explicit-origins tier isn't for.
pub const MAX_CSP_ORIGINS_PER_DIRECTIVE: usize = 16;

/// Preview/install-time content validation for `csp_extra_origins`, run
/// against the RAW manifest JSON (the struct parse deliberately ignores
/// unknown keys so old installs stay loadable; install must reject them).
///
/// Rejects: unknown directive keys, non-array values, non-string entries,
/// more than [`MAX_CSP_ORIGINS_PER_DIRECTIVE`] origins per directive, and
/// any entry that is not an explicit `https://host[:port]` origin.
pub fn validate_csp_extra_origins(raw_manifest_json: &str) -> Result<()> {
    let v: serde_json::Value = serde_json::from_str(raw_manifest_json)
        .map_err(|e| anyhow!("invalid manifest json: {e}"))?;
    let Some(csp) = v.get("csp_extra_origins") else {
        return Ok(());
    };
    if csp.is_null() {
        return Ok(());
    }
    let obj = csp
        .as_object()
        .ok_or_else(|| anyhow!("csp_extra_origins must be a JSON object"))?;
    for (key, val) in obj {
        if !CSP_EXTRA_DIRECTIVES.contains(&key.as_str()) {
            return Err(anyhow!(
                "csp_extra_origins: unknown directive {key:?} (allowed: {})",
                CSP_EXTRA_DIRECTIVES.join(", ")
            ));
        }
        let arr = val.as_array().ok_or_else(|| {
            anyhow!("csp_extra_origins.{key} must be an array of origin strings")
        })?;
        if arr.len() > MAX_CSP_ORIGINS_PER_DIRECTIVE {
            return Err(anyhow!(
                "csp_extra_origins.{key}: {} origins (max {MAX_CSP_ORIGINS_PER_DIRECTIVE})",
                arr.len()
            ));
        }
        for item in arr {
            let s = item.as_str().ok_or_else(|| {
                anyhow!("csp_extra_origins.{key} entries must be strings")
            })?;
            if !is_valid_csp_origin(s) {
                return Err(anyhow!(
                    "csp_extra_origins.{key}: {s:?} is not an explicit https origin \
                     (need https://host[:port] — lowercase, no wildcards, no CSP \
                     keywords, no schemes/paths/data:/blob:)"
                ));
            }
        }
    }
    Ok(())
}

/// Explicit https origin: `https://host[:port]` with lowercase DNS labels
/// (alnum + `-`, no leading/trailing `-`, joined by `.`) and an optional
/// port 1–65535. Everything else — wildcards, bare schemes ("https:"),
/// CSP keyword sources ('unsafe-eval', nonces, hashes), data:/blob:,
/// non-https schemes, paths/queries/fragments/userinfo, uppercase — fails.
fn is_valid_csp_origin(s: &str) -> bool {
    let Some(rest) = s.strip_prefix("https://") else {
        return false;
    };
    let (host, port) = match rest.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (rest, None),
    };
    if host.is_empty() {
        return false;
    }
    let host_ok = host.split('.').all(|label| {
        !label.is_empty()
            && label
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    });
    if !host_ok {
        return false;
    }
    match port {
        None => true,
        Some(p) => {
            !p.is_empty()
                && p.chars().all(|c| c.is_ascii_digit())
                && p.parse::<u32>().is_ok_and(|n| (1..=65535).contains(&n))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_manifest_succeeds() {
        let json = r#"{
            "id": "discord",
            "name": "Discord Bridge",
            "version": "0.1.0",
            "entry": "index.html",
            "requested_capabilities": ["cl_index_search"]
        }"#;
        let m = PluginManifest::parse(json).unwrap();
        assert_eq!(m.id, "discord");
        assert_eq!(m.iframe_origin(), "https://plugin-discord.localhost");
    }

    #[test]
    fn parse_rejects_uppercase_id() {
        let json = r#"{
            "id": "Discord",
            "name": "x",
            "version": "0.1.0",
            "entry": "i.html",
            "requested_capabilities": []
        }"#;
        let err = PluginManifest::parse(json).unwrap_err();
        assert!(err.to_string().contains("lowercase"));
    }

    #[test]
    fn parse_rejects_id_with_dots() {
        let json = r#"{
            "id": "my.plugin",
            "name": "x",
            "version": "0.1.0",
            "entry": "i.html",
            "requested_capabilities": []
        }"#;
        assert!(PluginManifest::parse(json).is_err());
    }

    #[test]
    fn parse_rejects_id_starting_with_hyphen() {
        let json = r#"{
            "id": "-bad",
            "name": "x",
            "version": "0.1.0",
            "entry": "i.html",
            "requested_capabilities": []
        }"#;
        assert!(PluginManifest::parse(json).is_err());
    }

    #[test]
    fn parse_rejects_capability_with_space() {
        let json = r#"{
            "id": "x",
            "name": "x",
            "version": "0.1.0",
            "entry": "i.html",
            "requested_capabilities": ["cl_index search"]
        }"#;
        assert!(PluginManifest::parse(json).is_err());
    }

    #[test]
    fn parse_rejects_empty_slot() {
        let json = r#"{
            "id": "x",
            "name": "x",
            "version": "0.1.0",
            "entry": "i.html",
            "requested_capabilities": [],
            "slots": [ { "slot_name": null, "panel_route": null } ]
        }"#;
        assert!(PluginManifest::parse(json).is_err());
    }

    #[test]
    fn parse_defaults_api_version_to_1_and_rejects_others() {
        let json = r#"{
            "id": "x",
            "name": "x",
            "version": "0.1.0",
            "entry": "i.html"
        }"#;
        assert_eq!(PluginManifest::parse(json).unwrap().api_version, 1);

        let json_v2 = r#"{
            "id": "x",
            "name": "x",
            "version": "0.1.0",
            "entry": "i.html",
            "api_version": 2
        }"#;
        let err = PluginManifest::parse(json_v2).unwrap_err();
        assert!(err.to_string().contains("api_version"));
    }

    #[test]
    fn parse_csp_extra_origins_field() {
        let json = r#"{
            "id": "cdn-user",
            "name": "x",
            "version": "0.1.0",
            "entry": "i.html",
            "csp_extra_origins": {
                "script-src": ["https://cdn.jsdelivr.net", "https://unpkg.com"],
                "font-src": ["https://fonts.gstatic.com"]
            }
        }"#;
        let m = PluginManifest::parse(json).unwrap();
        let csp = m.csp_extra_origins.unwrap();
        assert_eq!(
            csp.script_src,
            vec!["https://cdn.jsdelivr.net", "https://unpkg.com"]
        );
        assert_eq!(csp.font_src, vec!["https://fonts.gstatic.com"]);
        assert!(csp.style_src.is_empty() && csp.img_src.is_empty());
        assert!(!csp.is_empty());
    }

    #[test]
    fn parse_without_csp_field_is_none() {
        let json = r#"{
            "id": "x", "name": "x", "version": "0.1.0", "entry": "i.html"
        }"#;
        assert!(PluginManifest::parse(json).unwrap().csp_extra_origins.is_none());
    }

    /// Parse TOLERATES unknown directive keys (old installs must keep
    /// loading after a host upgrade) — install-time validation rejects them.
    #[test]
    fn parse_ignores_unknown_csp_directive_but_install_validation_rejects() {
        let json = r#"{
            "id": "x", "name": "x", "version": "0.1.0", "entry": "i.html",
            "csp_extra_origins": { "connect-src": ["https://evil.example"] }
        }"#;
        let m = PluginManifest::parse(json).unwrap();
        assert!(m.csp_extra_origins.unwrap().is_empty());

        let err = validate_csp_extra_origins(json).unwrap_err();
        assert!(err.to_string().contains("unknown directive"), "{err}");
    }

    #[test]
    fn csp_validation_accepts_absent_null_and_good_origins() {
        assert!(validate_csp_extra_origins(
            r#"{"id":"x","name":"x","version":"1","entry":"i.html"}"#
        )
        .is_ok());
        assert!(validate_csp_extra_origins(r#"{"csp_extra_origins": null}"#).is_ok());
        assert!(validate_csp_extra_origins(
            r#"{"csp_extra_origins": {
                "script-src": ["https://cdn.jsdelivr.net", "https://x.co:8443"],
                "style-src": ["https://fonts.googleapis.com"],
                "font-src": ["https://fonts.gstatic.com"],
                "img-src": ["https://raw.githubusercontent.com"]
            }}"#
        )
        .is_ok());
    }

    #[test]
    fn csp_validation_rejects_every_forbidden_origin_form() {
        for bad in [
            "https:",                     // blanket scheme
            "https://",                   // no host
            "*",                          // wildcard
            "https://*.example.com",      // wildcard subdomain
            "'unsafe-eval'",              // keyword source
            "'wasm-unsafe-eval'",         // keyword source
            "'nonce-abc123'",             // nonce
            "'sha256-deadbeef'",          // hash
            "data:",                      // data scheme
            "blob:",                      // blob scheme
            "http://insecure.example",    // non-https
            "ws://socket.example",        // non-https scheme
            "https://x.com/path",         // path
            "https://x.com?q=1",          // query
            "https://x.com#frag",         // fragment
            "https://user@x.com",         // userinfo
            "https://X.com",              // uppercase
            "https://x..com",             // empty label
            "https://-x.com",             // leading hyphen label
            "https://x.com:",             // empty port
            "https://x.com:0",            // port 0
            "https://x.com:70000",        // port out of range
            "https://x.com:8a",           // non-numeric port
            "https://x.com x",            // whitespace
            "",                           // empty
        ] {
            let json = format!(
                r#"{{"csp_extra_origins": {{ "script-src": ["{}"] }}}}"#,
                bad.replace('"', "\\\"")
            );
            assert!(
                validate_csp_extra_origins(&json).is_err(),
                "origin {bad:?} must be rejected"
            );
        }
    }

    #[test]
    fn csp_validation_rejects_bad_shapes_and_caps() {
        // Non-object field.
        assert!(validate_csp_extra_origins(r#"{"csp_extra_origins": 42}"#).is_err());
        // Non-array directive value.
        assert!(validate_csp_extra_origins(
            r#"{"csp_extra_origins": {"script-src": "https://x.com"}}"#
        )
        .is_err());
        // Non-string entry.
        assert!(validate_csp_extra_origins(
            r#"{"csp_extra_origins": {"script-src": [42]}}"#
        )
        .is_err());
        // Over the per-directive cap.
        let many: Vec<String> = (0..MAX_CSP_ORIGINS_PER_DIRECTIVE + 1)
            .map(|i| format!("\"https://cdn{i}.example.com\""))
            .collect();
        let json = format!(
            r#"{{"csp_extra_origins": {{ "img-src": [{}] }}}}"#,
            many.join(",")
        );
        assert!(validate_csp_extra_origins(&json).is_err());
    }

    #[test]
    fn csp_origins_roundtrip_via_serde_renames() {
        let csp = CspExtraOrigins {
            script_src: vec!["https://cdn.jsdelivr.net".into()],
            ..Default::default()
        };
        let json = serde_json::to_string(&csp).unwrap();
        assert!(json.contains("script-src"), "{json}");
        assert!(!json.contains("style-src"), "empty vecs skipped: {json}");
        let back: CspExtraOrigins = serde_json::from_str(&json).unwrap();
        assert_eq!(back, csp);
    }

    #[test]
    fn iframe_origin_uses_plugin_prefix() {
        let m = PluginManifest {
            id: "clive".to_string(),
            name: "Clive".to_string(),
            version: "0.1.0".to_string(),
            entry: "main.html".to_string(),
            api_version: 1,
            requested_capabilities: vec![],
            slots: vec![],
            csp_extra_origins: None,
        };
        assert_eq!(m.iframe_origin(), "https://plugin-clive.localhost");
    }
}
