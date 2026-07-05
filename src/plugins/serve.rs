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

/// Resolve an asset request against a PRE-RESOLVED serve root.
///
/// `root` is the caller's per-plugin serve-root lookup (normal installs:
/// `<data_dir>/plugins/<id>`; linked installs: the user's source directory
/// — the guards below treat WHATEVER root they're given as the boundary, so
/// a linked repo gets identical traversal/symlink protection). `None` means
/// the caller knows no such plugin. `enabled` is the registry's
/// enabled-cache verdict.
pub fn resolve_with_root(
    root: Option<&Path>,
    enabled: bool,
    plugin_id: &str,
    rel_path: &str,
) -> Result<(PathBuf, &'static str), ServeError> {
    if !is_safe_id(plugin_id) {
        return Err(ServeError::BadRequest);
    }
    let Some(plugin_dir) = root else {
        return Err(ServeError::UnknownPlugin);
    };
    if !enabled {
        return Err(ServeError::Disabled);
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
    // Symlinks inside the root pointing outside it resolve past `base` here
    // and get refused — that's the point of canonicalizing both sides. For
    // linked installs this is the load-bearing boundary: the root lives in
    // the user's filesystem, and nothing outside it is ever servable.
    if !candidate.starts_with(&base) {
        return Err(ServeError::Traversal);
    }
    if !candidate.is_file() {
        return Err(ServeError::NotFound);
    }
    Ok((candidate.clone(), mime_for(&candidate)))
}

/// Normal-mode convenience used by the scheme handler until the serve-root
/// cache lands (and by tests as the copied-bundle path): root is
/// `<plugins_root>/<id>` when that directory exists — preserving the
/// Disabled-vs-Unknown distinction (dir present but not enabled = Disabled).
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
    let root = plugin_dir.is_dir().then_some(plugin_dir.as_path());
    resolve_with_root(root, enabled_ids.contains(plugin_id), plugin_id, rel_path)
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
/// tab archetype) but everything else stays same-origin. Plugins can widen
/// four directives with consent-granted extra origins ([`build_plugin_csp`];
/// contract in docs/PLUGINS.md) — a plugin without that grant gets exactly
/// this header.
pub const PLUGIN_CSP: &str = "default-src 'self'; script-src 'self' 'unsafe-inline'; \
     style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; \
     font-src 'self' data:; connect-src *";

/// Build the CSP header for one plugin: the default policy with any
/// consent-granted extra origins APPENDED to their directives' source
/// lists. The defaults are never replaced or narrowed; `default-src` and
/// `connect-src` are never touched. `None` — and an all-empty grant —
/// yield [`PLUGIN_CSP`] byte-for-byte (tests assert it).
pub fn build_plugin_csp(extra: Option<&crate::plugins::CspExtraOrigins>) -> String {
    let empty = crate::plugins::CspExtraOrigins::default();
    let e = extra.unwrap_or(&empty);
    format!(
        "default-src 'self'; script-src 'self' 'unsafe-inline'{}; \
         style-src 'self' 'unsafe-inline'{}; img-src 'self' data: blob:{}; \
         font-src 'self' data:{}; connect-src *",
        join_origins(&e.script_src),
        join_origins(&e.style_src),
        join_origins(&e.img_src),
        join_origins(&e.font_src),
    )
}

/// ` https://a https://b` — leading space per origin so an empty list
/// contributes nothing to the directive.
fn join_origins(origins: &[String]) -> String {
    origins.iter().map(|o| format!(" {o}")).collect()
}

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

    /// resolve_with_root treats an ARBITRARY root (a linked repo living
    /// anywhere in the user's filesystem) as the serve boundary with the
    /// exact same guards as a copied bundle.
    #[test]
    fn linked_root_serves_and_guards_identically() {
        let tmp = TempDir::new().unwrap();
        // A "repo" far away from any plugins_root tree.
        let repo = tmp.path().join("projects").join("cognotify");
        std::fs::create_dir_all(repo.join("materials")).unwrap();
        std::fs::write(repo.join("index.html"), "<h1>hi</h1>").unwrap();
        std::fs::write(repo.join("materials").join("m1.html"), "<p>m</p>").unwrap();

        let (path, mime) =
            resolve_with_root(Some(&repo), true, "cognotify", "index.html").unwrap();
        assert!(path.ends_with("index.html"));
        assert_eq!(mime, "text/html; charset=utf-8");
        let (_, m) =
            resolve_with_root(Some(&repo), true, "cognotify", "materials/m1.html").unwrap();
        assert_eq!(m, "text/html; charset=utf-8");

        // ../ escapes refused at the linked boundary.
        for bad in ["../secret.txt", "materials/../../cognotify/index.html", "a/../../x"] {
            assert_eq!(
                resolve_with_root(Some(&repo), true, "cognotify", bad).unwrap_err(),
                ServeError::Traversal,
                "path {bad:?} must be refused at a linked root"
            );
        }

        // Unknown vs disabled come from the caller's map/enabled verdicts.
        assert_eq!(
            resolve_with_root(None, true, "ghost", "index.html").unwrap_err(),
            ServeError::UnknownPlugin
        );
        assert_eq!(
            resolve_with_root(Some(&repo), false, "cognotify", "index.html").unwrap_err(),
            ServeError::Disabled
        );
    }

    /// A symlink INSIDE the linked repo pointing OUTSIDE it is refused —
    /// the repo is the boundary even though it lives outside data_dir.
    #[cfg(unix)]
    #[test]
    fn linked_root_refuses_outward_symlink() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let outside = tmp.path().join("host-secret.txt");
        std::fs::write(&outside, "s3cret").unwrap();
        std::os::unix::fs::symlink(&outside, repo.join("leak.txt")).unwrap();
        assert_eq!(
            resolve_with_root(Some(&repo), true, "repo", "leak.txt").unwrap_err(),
            ServeError::Traversal
        );
    }

    #[test]
    fn build_csp_without_grant_is_byte_identical_to_default() {
        use crate::plugins::CspExtraOrigins;
        assert_eq!(build_plugin_csp(None), PLUGIN_CSP);
        // An all-empty grant is semantically "no grant" — same bytes.
        assert_eq!(build_plugin_csp(Some(&CspExtraOrigins::default())), PLUGIN_CSP);
    }

    #[test]
    fn build_csp_appends_origins_to_their_directives_keeping_defaults() {
        use crate::plugins::CspExtraOrigins;
        let extra = CspExtraOrigins {
            script_src: vec![
                "https://cdn.jsdelivr.net".into(),
                "https://unpkg.com".into(),
            ],
            style_src: vec!["https://fonts.googleapis.com".into()],
            font_src: vec!["https://fonts.gstatic.com".into()],
            img_src: vec!["https://raw.githubusercontent.com".into()],
        };
        let csp = build_plugin_csp(Some(&extra));
        assert_eq!(
            csp,
            "default-src 'self'; \
             script-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net https://unpkg.com; \
             style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; \
             img-src 'self' data: blob: https://raw.githubusercontent.com; \
             font-src 'self' data: https://fonts.gstatic.com; \
             connect-src *"
        );
    }

    #[test]
    fn build_csp_partial_grant_touches_only_named_directives() {
        use crate::plugins::CspExtraOrigins;
        let extra = CspExtraOrigins {
            script_src: vec!["https://cdn.jsdelivr.net".into()],
            ..Default::default()
        };
        let csp = build_plugin_csp(Some(&extra));
        assert!(csp.contains("script-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net;"));
        // Untouched directives keep their default source lists exactly.
        assert!(csp.contains("style-src 'self' 'unsafe-inline';"));
        assert!(csp.contains("img-src 'self' data: blob:;"));
        assert!(csp.contains("font-src 'self' data:;"));
        assert!(csp.starts_with("default-src 'self';"));
        assert!(csp.ends_with("connect-src *"));
    }
}
