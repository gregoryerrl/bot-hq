//! Capture the bot-hq main window to a PNG for the `webview_screenshot`
//! MCP tool (agent-triggered "eyes on the UI").
//!
//! # Platform support
//!
//! This shipped in 1.0.0 as a macOS-only implementation with no `cfg` gate at
//! all — `/usr/sbin/screencapture` hardcoded, called unconditionally from
//! `signaling/jsonrpc.rs`. On Linux and Windows the tool therefore failed at
//! RUNTIME ("screencapture spawn"), which no build could catch, and its error
//! text then told the user to open a macOS Settings pane that does not exist on
//! their OS. Found on Fedora 2026-08-25 by an agent trying to screenshot a
//! rendering bug on the user's own machine.
//!
//! Now: macOS keeps `screencapture`; Linux picks the first available of
//! Spectacle / grim / ImageMagick `import` / gnome-screenshot; Windows says so
//! plainly instead of pretending.
//!
//! # What the result guard does and does not cover
//!
//! Every region-based backend — including macOS `screencapture -R`, so this
//! predates the Linux work — captures whatever pixels are AT the coordinates,
//! not the window that owns them. If another window overlaps bot-hq and the
//! best-effort raise below did not take, the capture is of that window, at
//! exactly the right size. [`plausible_capture`] therefore catches wrong
//! GEOMETRY and never wrong CONTENT; occlusion is mitigated solely by the
//! raise. The durable fix is to target the window directly (`import -window
//! <id>`), tracked on the changelog's deferred list rather than half-done here.
//!
//! The window geometry is Tauri's (physical pixels ÷ scale factor —
//! Retina-safe). The PNG lands under `<data_dir>/.local/screenshots/<ts>.png`;
//! the caller reads the file back as an image.

use anyhow::Context;
use chrono::Utc;
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::Manager;

/// The window rectangle in LOGICAL screen coordinates, which is what every
/// capture backend here takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Region {
    pub x: i64,
    pub y: i64,
    pub w: u64,
    pub h: u64,
}

/// A resolved capture invocation: program plus argv.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Capture {
    pub program: String,
    pub args: Vec<String>,
    /// True when the backend applies the RECTANGLE it was given (`grim -g`,
    /// `import -crop`), false when it returns whatever window is active
    /// (`spectacle -a`, `gnome-screenshot -w`). This changes how the result is
    /// size-checked — see [`plausible_capture`] — and nothing more. It does
    /// NOT mean the pixels are ours: `import -window root -crop` grabs the
    /// composited desktop and cuts our rectangle out of it, so an overlapping
    /// window lands in the crop at exactly the requested size.
    pub geometry: bool,
}

impl Capture {
    fn new(program: &str, args: &[String], geometry: bool) -> Self {
        Self {
            program: program.to_string(),
            args: args.to_vec(),
            geometry,
        }
    }
}

/// Capture the main bot-hq window to a PNG under `<data_dir>/.local/screenshots/`.
/// Used by the `webview_screenshot` MCP tool (agent-triggered) — returns the
/// path; the agent reads the PNG back as an image.
pub(crate) fn capture_main_window(
    app_handle: &tauri::AppHandle,
    data_dir: &Path,
) -> anyhow::Result<PathBuf> {
    let window = app_handle
        .get_webview_window("main")
        .context("main window not found")?;

    // Every backend here captures whatever pixels are at the given screen
    // coordinates (or whatever window is ACTIVE, for Spectacle) — including
    // anything stacked on top of bot-hq. Raise it first so we capture its
    // actual contents instead of whatever overlay (terminal, devtools, another
    // app) happens to be covering it. Brief sleep lets the compositor redraw.
    // The focus RESULT is kept, not discarded: on Linux the Spectacle backend
    // captures the ACTIVE window, so a refused focus request is the leading
    // explanation when the capture turns out not to be ours. Compositors
    // routinely refuse focus-stealing — native Wayland almost always.
    let focus = window.set_focus();
    std::thread::sleep(std::time::Duration::from_millis(150));

    let pos = window.outer_position().context("outer_position")?;
    let size = window.outer_size().context("outer_size")?;
    let scale = window.scale_factor().context("scale_factor")?;

    let region = Region {
        x: (pos.x as f64 / scale).round() as i64,
        y: (pos.y as f64 / scale).round() as i64,
        w: (size.width as f64 / scale).round() as u64,
        h: (size.height as f64 / scale).round() as u64,
    };

    let dir = crate::paths::Paths::for_data_dir(data_dir.to_path_buf()).screenshots_dir;
    std::fs::create_dir_all(&dir).context("mkdir screenshots dir")?;

    let ts = Utc::now().format("%Y%m%dT%H%M%S%3f").to_string();
    let path = dir.join(format!("{ts}.png"));

    // Physical size too: a capture is in device pixels, `region` is logical.
    let physical = (size.width, size.height);
    run_capture(region, physical, focus.is_ok(), &path)?;
    Ok(path)
}

// ---------------------------------------------------------------- macOS ----

#[cfg(target_os = "macos")]
fn run_capture(
    region: Region,
    _physical: (u32, u32),
    _focused: bool,
    path: &Path,
) -> anyhow::Result<()> {
    let rect = format!("{},{},{},{}", region.x, region.y, region.w, region.h);
    let output = Command::new("/usr/sbin/screencapture")
        .args(["-R", &rect, "-x", "-t", "png"])
        .arg(path)
        .output()
        .context("screencapture spawn")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Screen Recording permission missing → "could not create image from
        // display/rect". Translate into something actionable. This text is
        // macOS-only BY CONSTRUCTION now — it used to reach Linux and Windows
        // users, telling them to open a Settings pane their OS does not have.
        if stderr.contains("could not create image") {
            anyhow::bail!(
                "Screen Recording permission required. Open System Settings → \
                 Privacy & Security → Screen Recording, enable the entry for \
                 bot-hq (or your terminal if launched via `cargo run`), then \
                 try again. Raw output: {}",
                stderr.trim()
            );
        }
        anyhow::bail!("screencapture failed: {}", stderr.trim());
    }
    Ok(())
}

// ---------------------------------------------------------------- Linux ----

/// The backends, in preference order, as (program, argv builder).
///
/// Spectacle first: it is KDE's own and works on both X11 and Wayland, where
/// `import` is X11-only and `grim` is wlroots-only. `gnome-screenshot` last —
/// it is deprecated upstream but still present on older GNOME installs.
#[cfg(target_os = "linux")]
pub(crate) fn linux_captures(
    available: impl Fn(&str) -> bool,
    region: Region,
    path: &Path,
) -> Vec<Capture> {
    // Cloned per candidate: every backend now lands in the list rather than
    // returning early, so a single moved String no longer works.
    let out = path.display().to_string();
    let Region { x, y, w, h } = region;
    let mut all = Vec::new();

    if available("spectacle") {
        // -b background (no GUI), -n no notification, -a active window.
        all.push(Capture::new(
            "spectacle",
            &["-b".into(), "-n".into(), "-a".into(), "-o".into(), out.clone()],
            false,
        ));
    }
    if available("grim") {
        all.push(Capture::new(
            "grim",
            &["-g".into(), format!("{x},{y} {w}x{h}"), out.clone()],
            true,
        ));
    }
    if available("import") {
        // ImageMagick: grab the root window, then crop to ours. `+repage`
        // drops the crop offset so the PNG is a plain w×h image.
        //
        // MEASURED LIMIT (Fedora 44 / KDE Wayland, ImageMagick 7.1.2): this
        // ROOT-window form fails here — `import: missing an image filename`,
        // no file written — with a real filename supplied. It is not name
        // resolution: passing the numeric root id from `xdpyinfo` (0x400)
        // fails identically, while `-window <ordinary window id>` on the same
        // binary succeeds. So root capture specifically does not work here and
        // the error text is misleading; WHY is not established, and guessing
        // (XWayland, portals) would just be reading a mechanism off an error
        // message that does not mention one.
        //
        // Kept because the root grab is correct on a real X11 session, ordered
        // after Spectacle, and the fall-through means its failure no longer
        // ends the attempt. The window-id form needs an XID the code does not
        // have; the changelog's deferred list tracks that, and it would fix occlusion too.
        all.push(Capture::new(
            "import",
            &[
                "-window".into(),
                "root".into(),
                "-crop".into(),
                format!("{w}x{h}+{x}+{y}"),
                "+repage".into(),
                out.clone(),
            ],
            true,
        ));
    }
    if available("gnome-screenshot") {
        all.push(Capture::new(
            "gnome-screenshot",
            &["-w".into(), "-f".into(), out],
            false,
        ));
    }
    all
}

#[cfg(target_os = "linux")]
fn on_path(program: &str) -> bool {
    // `which` without the shell: walk PATH ourselves so a scrubbed or unusual
    // environment does not change the answer.
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                let candidate = dir.join(program);
                candidate.is_file()
            })
        })
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn run_capture(
    region: Region,
    physical: (u32, u32),
    focused: bool,
    path: &Path,
) -> anyhow::Result<()> {
    let candidates = linux_captures(on_path, region, path);
    if candidates.is_empty() {
        anyhow::bail!(
            "no screenshot backend found. Install one of: spectacle (KDE), grim \
             (wlroots/Sway), ImageMagick (`import`), or gnome-screenshot."
        );
    }
    // Try each in turn rather than only the first. A backend can be PRESENT
    // and inapplicable — measured here: ImageMagick's root grab fails under
    // XWayland while Spectacle succeeds on the same host — so "found on PATH"
    // is not "will work", and stopping at the first would surface an error
    // when a working backend was installed all along.
    let mut failures = Vec::new();
    for capture in &candidates {
        match try_capture(capture, region, physical, focused, path) {
            Ok(()) => return Ok(()),
            Err(e) => failures.push(format!("{}: {e}", capture.program)),
        }
    }
    anyhow::bail!("every screenshot backend failed —\n  {}", failures.join("\n  "))
}

#[cfg(target_os = "linux")]
fn try_capture(
    capture: &Capture,
    region: Region,
    physical: (u32, u32),
    focused: bool,
    path: &Path,
) -> anyhow::Result<()> {
    // Start from no file. Every candidate writes to the SAME path, and a
    // backend can exit nonzero having written a partial one (a portal denial
    // mid-write) or exit ZERO having written nothing — `import` does exactly
    // that here. Without this, the next backend's `len() > 0` and size checks
    // would read the PREVIOUS backend's leftover bytes and report success,
    // attributing one backend's image to another.
    let _ = std::fs::remove_file(path);

    let mut cmd = Command::new(&capture.program);
    cmd.args(&capture.args);
    // Under an AppImage launch the payload's library paths would follow the
    // backend into its own process. Measured on Fedora: Spectacle dies on
    // LD_LIBRARY_PATH — `liblzma.so.5: version XZ_5.4 not found (required by
    // libKF6Archive.so.6)` and `libgnutls.so.30: version GNUTLS_3_7_7 not
    // found` — NOT on QT_PLUGIN_PATH, which is harmless to it. Two more host
    // libraries the payload shadows. See `appimage_env`.
    crate::appimage_env::scrub_std(&mut cmd);

    let output = cmd
        .output()
        .with_context(|| format!("{} spawn", capture.program))?;

    if !output.status.success() {
        anyhow::bail!(
            "{} failed: {}",
            capture.program,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    // Spectacle and gnome-screenshot capture the ACTIVE WINDOW rather than a
    // rectangle, and report success even when they wrote nothing (no active
    // window, portal denied) — or, worse, when they wrote SOMEBODY ELSE's
    // window because the compositor refused our focus request. A non-empty
    // file rules out the first failure and not the second, and the second is
    // the dangerous one: the consumer is an agent, which cannot tell it is
    // looking at the wrong application and will report on it confidently.
    let bytes = std::fs::read(path).unwrap_or_default();
    if bytes.is_empty() {
        anyhow::bail!(
            "{} reported success but wrote no image to {}",
            capture.program,
            path.display()
        );
    }
    let Some(got) = png_dimensions(&bytes) else {
        anyhow::bail!("{} wrote a file that is not a PNG", capture.program);
    };
    if !plausible_capture(got, region, physical, capture.geometry) {
        let _ = std::fs::remove_file(path);
        anyhow::bail!(
            "{} captured a {}x{} image, but the bot-hq window is {}x{} logical \
             ({}x{} physical) — this is very likely a DIFFERENT window. \
             {}Raise bot-hq to the front and try again.",
            capture.program,
            got.0,
            got.1,
            region.w,
            region.h,
            physical.0,
            physical.1,
            if capture.geometry {
                "A geometry backend returned a LARGER image than the window, \
                 which usually means the window moved mid-capture. "
            } else if focused {
                ""
            } else {
                "The focus request was refused by the compositor, which is the \
                 usual cause. "
            }
        );
    }
    Ok(())
}

/// Width and height from a PNG's IHDR chunk — fixed offsets, no dependency.
///
/// IHDR is mandated to be the FIRST chunk, so width and height always live at
/// bytes 16..24: 8-byte signature, 4-byte length, 4-byte type, then two
/// big-endian u32s.
pub(crate) fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((w, h))
}

/// Could an image of size `got` plausibly BE the bot-hq window?
///
/// Two modes, because the backends differ in kind. A geometry backend
/// (`geometry`) is held to the rectangle it was given — never larger, and
/// smaller only because the compositor clipped it at a screen edge. An
/// active-window backend cannot be held to anything, and gets the loose band
/// below.
///
/// Deliberately loose for the latter, because such a capture is not pixel-exact: window
/// decorations and compositor shadows add to an active-window grab, and a
/// geometry-based backend on a HiDPI screen returns device pixels where
/// `region` is logical. So the test is proportional, not exact — both
/// dimensions within [0.75, 1.5] of either the logical or the physical size,
/// AND an aspect ratio within 25%.
///
/// In the loose mode this cannot prove identity, and its reach is narrower
/// than it looks: measured on a 1920x1080 screen with a 1920x1006 window, a
/// FULL-SCREEN grab passes every test (ratios 1.00 and 1.07, aspect within
/// 7%). A maximized window is nearly the shape of the screen, so the guard is
/// weakest exactly where the app is most often run. What it reliably catches
/// is a small or dialog-shaped wrong window. Stated here and in
/// `docs/FEDORA-LINUX-COMPAT.md` rather than pretended away.
pub(crate) fn plausible_capture(
    got: (u32, u32),
    region: Region,
    physical: (u32, u32),
    geometry: bool,
) -> bool {
    let logical = (region.w as u32, region.h as u32);
    if geometry {
        // `grim -g` and `import -crop` apply our rectangle to the whole
        // screen, so they get the GEOMETRY right by construction — but not the
        // CONTENT: whatever is stacked on top of bot-hq is what lies at those
        // coordinates, and it arrives at exactly the requested size. Occlusion
        // is mitigated only by the best-effort raise above, on every backend.
        // What this check covers is wrong geometry, never wrong content.
        //
        // They may legitimately return LESS than asked for: ImageMagick clips rather than erroring when the
        // rectangle runs off an edge (measured: `-crop 50x50+80+0` on a 100x100
        // root yields 20x50), which happens whenever the window is dragged
        // partly off-screen or sits at a negative x on a multi-monitor layout.
        // A clipped capture of the RIGHT window is still useful, so the rule is
        // "never LARGER than requested" rather than "exactly equal" — demanding
        // equality would refuse an ordinary dragged window and blame it on a
        // wrong-window failure that did not happen (finding 2b65c105).
        let within =
            |want: (u32, u32)| got.0 > 0 && got.1 > 0 && got.0 <= want.0 && got.1 <= want.1;
        return within(logical) || within(physical);
    }
    let fits = |want: (u32, u32)| {
        if want.0 == 0 || want.1 == 0 || got.0 == 0 || got.1 == 0 {
            return false;
        }
        let rw = got.0 as f64 / want.0 as f64;
        let rh = got.1 as f64 / want.1 as f64;
        let in_range = |r: f64| (0.75..=1.5).contains(&r);
        if !in_range(rw) || !in_range(rh) {
            return false;
        }
        let got_aspect = got.0 as f64 / got.1 as f64;
        let want_aspect = want.0 as f64 / want.1 as f64;
        (got_aspect - want_aspect).abs() / want_aspect <= 0.25
    };
    fits(logical) || fits(physical)
}

// -------------------------------------------------------------- Windows ----

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn run_capture(
    _region: Region,
    _physical: (u32, u32),
    _focused: bool,
    _path: &Path,
) -> anyhow::Result<()> {
    anyhow::bail!(
        "webview_screenshot is not implemented on this platform \
         ({}). Take the screenshot manually and share the file instead.",
        std::env::consts::OS
    )
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    const R: Region = Region {
        x: 100,
        y: 50,
        w: 1280,
        h: 800,
    };

    fn args_of(c: &Capture) -> Vec<&str> {
        c.args.iter().map(String::as_str).collect()
    }

    /// The single candidate a one-tool `available` predicate produces.
    fn only(mut v: Vec<Capture>) -> Capture {
        assert_eq!(v.len(), 1, "expected exactly one candidate, got {v:?}");
        v.remove(0)
    }

    #[test]
    fn prefers_spectacle_when_present() {
        let c = only(linux_captures(|p| p == "spectacle", R, Path::new("/tmp/s.png")));
        assert_eq!(c.program, "spectacle");
        assert_eq!(args_of(&c), ["-b", "-n", "-a", "-o", "/tmp/s.png"]);
    }

    #[test]
    fn grim_gets_the_wayland_geometry_string() {
        let c = only(linux_captures(|p| p == "grim", R, Path::new("/tmp/s.png")));
        assert_eq!(c.program, "grim");
        assert_eq!(args_of(&c), ["-g", "100,50 1280x800", "/tmp/s.png"]);
    }

    #[test]
    fn import_crops_the_root_window_to_ours() {
        let c = only(linux_captures(|p| p == "import", R, Path::new("/tmp/s.png")));
        assert_eq!(c.program, "import");
        assert_eq!(
            args_of(&c),
            ["-window", "root", "-crop", "1280x800+100+50", "+repage", "/tmp/s.png"]
        );
    }

    #[test]
    fn falls_through_to_gnome_screenshot_last() {
        let c = only(linux_captures(|p| p == "gnome-screenshot", R, Path::new("/tmp/s.png")));
        assert_eq!(c.program, "gnome-screenshot");
        assert_eq!(args_of(&c), ["-w", "-f", "/tmp/s.png"]);
    }

    #[test]
    fn preference_order_holds_when_several_are_present() {
        let names: Vec<String> = linux_captures(|_| true, R, Path::new("/tmp/s.png"))
            .into_iter()
            .map(|c| c.program)
            .collect();
        // All four are offered, in order — the runner falls through to the
        // next when one fails, so the list matters, not just its head.
        assert_eq!(names, ["spectacle", "grim", "import", "gnome-screenshot"]);
        let no_spectacle: Vec<String> = linux_captures(|p| p != "spectacle", R, Path::new("/tmp/s.png"))
            .into_iter()
            .map(|c| c.program)
            .collect();
        assert_eq!(no_spectacle, ["grim", "import", "gnome-screenshot"]);
    }

    // --- capture validation (the wrong-window failure) --------------------

    const PHYS: (u32, u32) = (1280, 800);

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        v.extend_from_slice(&13u32.to_be_bytes());
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v
    }

    #[test]
    fn reads_dimensions_from_a_png_header() {
        assert_eq!(png_dimensions(&png(1280, 800)), Some((1280, 800)));
    }

    #[test]
    fn rejects_a_non_png_and_a_truncated_one() {
        assert_eq!(png_dimensions(b"not a png at all really"), None);
        assert_eq!(png_dimensions(&png(1280, 800)[..20]), None);
        assert_eq!(png_dimensions(&[]), None);
    }

    #[test]
    fn an_exact_capture_is_plausible() {
        assert!(plausible_capture((1280, 800), R, PHYS, false));
    }

    #[test]
    fn decorations_and_shadows_stay_plausible() {
        // An active-window grab includes the frame; a little larger is fine.
        assert!(plausible_capture((1310, 840), R, PHYS, false));
    }

    #[test]
    fn a_hidpi_capture_matches_the_physical_size() {
        // region is logical; grim/import return device pixels.
        let logical = Region { x: 0, y: 0, w: 640, h: 400 };
        assert!(plausible_capture((1280, 800), logical, PHYS, false));
    }

    #[test]
    fn a_full_screen_grab_instead_of_the_window_is_rejected() {
        assert!(!plausible_capture((3840, 2160), R, PHYS, false));
    }

    #[test]
    fn a_different_window_of_a_different_shape_is_rejected() {
        // The failure this exists for: focus refused, Spectacle grabs the
        // terminal, the agent analyses the wrong application.
        assert!(!plausible_capture((900, 1600), R, PHYS, false));
    }

    #[test]
    fn a_geometry_backend_is_never_larger_than_requested() {
        // grim -g and import -crop apply our rectangle to the screen, so the
        // SIZE is theirs to get right; a CLIPPED result is ordinary, so
        // smaller is accepted. (Occlusion is a separate problem this check
        // cannot see — see plausible_capture.)
        assert!(plausible_capture((1280, 800), R, PHYS, true));
        // Larger than asked for → the rectangle was not honoured.
        assert!(!plausible_capture((1310, 840), R, PHYS, true));
        // SMALLER is fine: ImageMagick CLIPS a rectangle that runs off an edge,
        // so a window dragged half off-screen yields a short image of the RIGHT
        // window. Refusing it would blame a wrong-window failure that did not
        // happen (reviewer finding 2b65c105).
        assert!(plausible_capture((1279, 800), R, PHYS, true));
        assert!(plausible_capture((640, 200), R, PHYS, true));
        assert!(!plausible_capture((0, 0), R, PHYS, true));
    }

    #[test]
    fn a_geometry_backend_accepts_hidpi_device_pixels() {
        let logical = Region { x: 0, y: 0, w: 640, h: 400 };
        assert!(plausible_capture((1280, 800), logical, PHYS, true));
    }

    #[test]
    fn the_loose_band_admits_a_full_screen_grab_and_we_know_it() {
        // Measured on this host: 1920x1006 window on a 1920x1080 screen. The
        // ratios are 1.00 and 1.07 and the aspect is within 7%, so a
        // full-screen grab PASSES. This test pins the limit as KNOWN rather
        // than letting a future reader assume the guard proves identity.
        let win = Region { x: 0, y: 0, w: 1920, h: 1006 };
        assert!(plausible_capture((1920, 1080), win, (1920, 1006), false));
        // ...and the geometry backends are not fooled by it.
        assert!(!plausible_capture((1920, 1080), win, (1920, 1006), true));
    }

    #[test]
    fn backends_declare_whether_they_apply_our_geometry() {
        let path = Path::new("/tmp/s.png");
        assert!(!only(linux_captures(|p| p == "spectacle", R, path)).geometry);
        assert!(only(linux_captures(|p| p == "grim", R, path)).geometry);
        assert!(only(linux_captures(|p| p == "import", R, path)).geometry);
        assert!(!only(linux_captures(|p| p == "gnome-screenshot", R, path)).geometry);
    }

    #[test]
    fn a_zero_dimension_is_never_plausible() {
        assert!(!plausible_capture((0, 800), R, PHYS, false));
        assert!(!plausible_capture((1280, 0), R, PHYS, false));
        assert!(!plausible_capture((0, 0), R, PHYS, true));
    }

    #[test]
    fn none_available_yields_an_empty_list_not_a_panic() {
        assert!(linux_captures(|_| false, R, Path::new("/tmp/s.png")).is_empty());
    }
}
