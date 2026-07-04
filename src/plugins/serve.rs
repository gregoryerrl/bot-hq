//! Resolving plugin-bundle asset requests for the `bhq-plugin://` custom
//! URI scheme.
//!
//! URL shape: `bhq-plugin://<plugin-id>/<path>`. On macOS/Linux the webview
//! delivers the request with the plugin id in the URI HOST; on Windows the
//! scheme is surfaced through an `https://…localhost` origin and the id can
//! arrive as the first PATH segment instead — [`parse_plugin_request`]
//! accepts both forms so the handler is platform-agnostic.
//!
//! This module is pure std (no tauri dependency, mirroring the rest of
//! `plugins/`); the thin `tauri::http` glue lives in `main.rs`. Security
//! invariants enforced here:
//!
//! - only ENABLED plugins are served (callers pass the enabled-id set — the
//!   [`PluginRegistry`](crate::plugins::PluginRegistry) cache, seeded from
//!   the DB at boot)
//! - the resolved file must stay inside the plugin's bundle dir
//!   (canonicalize + prefix check kills `..` and symlink escapes)
//! - percent-encoded paths are rejected outright — bundle filenames must be
//!   URL-safe ASCII, which sidesteps decode-then-check bypasses

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Why an asset request was refused. Mapped to an HTTP status by the glue
/// in `main.rs` (404 for everything except `Disabled` → 403).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServeError {
    /// No plugin with that id is enabled AND no bundle dir exists.
    UnknownPlugin,
    /// The bundle dir exists but the plugin is not in the enabled set.
    Disabled,
    /// The path escapes the plugin's bundle dir.
    Traversal,
    /// Malformed id / path (bad characters, empty, percent-encoding).
    BadRequest,
    /// Inside the bundle dir but no such file.
    NotFound,
}

/// Split a request URI into `(plugin_id, relative_path)`.
///
/// Accepts both delivery forms:
/// - host form (macOS/Linux): `bhq-plugin://discord/index.html`
///   → host = `discord`, path = `/index.html`
/// - path form (Windows fold): host empty or `localhost`, id as the first
///   path segment: `/discord/index.html`
///
/// The query string (`?bhq=<nonce>`) is NOT part of either output — callers
/// pass `uri.host()` / `uri.path()`, which exclude it already.
pub fn parse_plugin_request<'a>(
    host: Option<&'a str>,
    path: &'a str,
) -> Result<(&'a str, &'a str), ServeError> {
    let path = path.strip_prefix('/').unwrap_or(path);
    match host {
        Some(h) if !h.is_empty() && h != "localhost" => Ok((h, path)),
        _ => match path.split_once('/') {
            Some((id, rest)) if !id.is_empty() => Ok((id, rest)),
            _ => Err(ServeError::BadRequest),
        },
    }
}

/// Resolve an asset request to an on-disk file + MIME type.
///
/// `plugins_root` is `<data_dir>/plugins/`; `enabled_ids` is the registry's
/// enabled-cache snapshot.
pub fn resolve_plugin_asset(
    plugins_root: &Path,
    enabled_ids: &HashSet<String>,
    plugin_id: &str,
    rel_path: &str,
) -> Result<(PathBuf, &'static str), ServeError> {
    if !is_safe_id(plugin_id) {
        return Err(ServeError::BadRequest);
    }
    let plugin_dir = plugins_root.join(plugin_id);
    if !enabled_ids.contains(plugin_id) {
        return if plugin_dir.is_dir() {
            Err(ServeError::Disabled)
        } else {
            Err(ServeError::UnknownPlugin)
        };
    }
    if rel_path.is_empty() {
        return Err(ServeError::BadRequest);
    }
    if !is_safe_path(rel_path) {
        return Err(ServeError::Traversal);
    }

    let base = plugin_dir.canonicalize().map_err(|_| ServeError::NotFound)?;
    let candidate = plugin_dir
        .join(rel_path)
        .canonicalize()
        .map_err(|_| ServeError::NotFound)?;
    // Symlinks inside the bundle pointing outside it resolve past `base`
    // here and get refused — that's the point of canonicalizing both sides.
    if !candidate.starts_with(&base) {
        return Err(ServeError::Traversal);
    }
    if !candidate.is_file() {
        return Err(ServeError::NotFound);
    }
    Ok((candidate.clone(), mime_for(&candidate)))
}

/// Same shape as manifest id validation (lowercase alnum + `-`), duplicated
/// as a serving-side guard so a crafted host can't smuggle path syntax.
fn is_safe_id(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

/// Reject before touching the filesystem: `..` segments, backslashes,
/// percent-encoding, NULs, absolute paths.
fn is_safe_path(p: &str) -> bool {
    if p.contains('%') || p.contains('\\') || p.contains('\0') || p.starts_with('/') {
        return false;
    }
    p.split('/').all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

/// MIME by extension. Unknown extensions serve as octet-stream — browsers
/// won't execute those, which is the safe default.
pub fn mime_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "wasm" => "application/wasm",
        "txt" | "md" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// The default Content-Security-Policy served with every plugin asset.
/// Permissive on connect-src (plugins may call external APIs — the GitHub
/// tab archetype) but everything else stays same-origin. Per-plugin CSP
/// overrides are a deferred tier (docs/PLUGINS.md).
pub const PLUGIN_CSP: &str = "default-src 'self'; script-src 'self' 'unsafe-inline'; \
     style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; \
     font-src 'self' data:; connect-src *";

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use tempfile::TempDir;

    fn setup(id: &str, enabled: bool) -> (TempDir, PathBuf, HashSet<String>) {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("plugins");
        let dir = root.join(id);
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(dir.join("index.html"), "<h1>hi</h1>").unwrap();
        std::fs::write(dir.join("assets").join("app.js"), "//js").unwrap();
        let mut set = HashSet::new();
        if enabled {
            set.insert(id.to_string());
        }
        (tmp, root, set)
    }

    #[test]
    fn parse_host_form() {
        assert_eq!(
            parse_plugin_request(Some("discord"), "/index.html").unwrap(),
            ("discord", "index.html")
        );
    }

    #[test]
    fn parse_path_form_when_host_is_localhost_or_empty() {
        assert_eq!(
            parse_plugin_request(Some("localhost"), "/discord/index.html").unwrap(),
            ("discord", "index.html")
        );
        assert_eq!(
            parse_plugin_request(None, "/discord/assets/app.js").unwrap(),
            ("discord", "assets/app.js")
        );
    }

    #[test]
    fn parse_rejects_bare_paths() {
        assert_eq!(
            parse_plugin_request(None, "/index.html").unwrap_err(),
            ServeError::BadRequest
        );
        assert_eq!(parse_plugin_request(None, "/").unwrap_err(), ServeError::BadRequest);
    }

    #[test]
    fn resolves_enabled_plugin_file() {
        let (_tmp, root, set) = setup("notes", true);
        let (path, mime) = resolve_plugin_asset(&root, &set, "notes", "index.html").unwrap();
        assert!(path.ends_with("index.html"));
        assert_eq!(mime, "text/html; charset=utf-8");
        let (_, mime_js) = resolve_plugin_asset(&root, &set, "notes", "assets/app.js").unwrap();
        assert_eq!(mime_js, "text/javascript; charset=utf-8");
    }

    #[test]
    fn refuses_disabled_plugin_but_distinguishes_unknown() {
        let (_tmp, root, set) = setup("notes", false);
        assert_eq!(
            resolve_plugin_asset(&root, &set, "notes", "index.html").unwrap_err(),
            ServeError::Disabled
        );
        assert_eq!(
            resolve_plugin_asset(&root, &set, "ghost", "index.html").unwrap_err(),
            ServeError::UnknownPlugin
        );
    }

    #[test]
    fn refuses_traversal_and_bad_paths() {
        let (_tmp, root, set) = setup("notes", true);
        for bad in [
            "../notes/index.html",
            "a/../../x",
            "assets/../../notes/index.html",
            "a%2e%2e/x",
            "a\\..\\x",
        ] {
            assert_eq!(
                resolve_plugin_asset(&root, &set, "notes", bad).unwrap_err(),
                ServeError::Traversal,
                "path {bad:?} should be refused"
            );
        }
        assert_eq!(
            resolve_plugin_asset(&root, &set, "notes", "").unwrap_err(),
            ServeError::BadRequest
        );
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlink_escaping_bundle_dir() {
        let (tmp, root, set) = setup("notes", true);
        let outside = tmp.path().join("secret.txt");
        std::fs::write(&outside, "s3cret").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("notes").join("leak.txt")).unwrap();
        assert_eq!(
            resolve_plugin_asset(&root, &set, "notes", "leak.txt").unwrap_err(),
            ServeError::Traversal
        );
    }

    #[test]
    fn refuses_bad_plugin_ids() {
        let (_tmp, root, set) = setup("notes", true);
        for bad in ["Notes", "no tes", "-x", "x-", "a/b", "..", ""] {
            assert_eq!(
                resolve_plugin_asset(&root, &set, bad, "index.html").unwrap_err(),
                ServeError::BadRequest,
                "id {bad:?} should be refused"
            );
        }
    }

    #[test]
    fn missing_file_inside_bundle_is_not_found() {
        let (_tmp, root, set) = setup("notes", true);
        assert_eq!(
            resolve_plugin_asset(&root, &set, "notes", "nope.js").unwrap_err(),
            ServeError::NotFound
        );
        // A directory is not a servable file either.
        assert_eq!(
            resolve_plugin_asset(&root, &set, "notes", "assets").unwrap_err(),
            ServeError::NotFound
        );
    }

    #[test]
    fn mime_map_covers_common_types_and_defaults_safe() {
        assert_eq!(mime_for(Path::new("x.wasm")), "application/wasm");
        assert_eq!(mime_for(Path::new("x.woff2")), "font/woff2");
        assert_eq!(mime_for(Path::new("x.weird")), "application/octet-stream");
        assert_eq!(mime_for(Path::new("noext")), "application/octet-stream");
    }
}
