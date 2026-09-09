//! Fail-loud webview startup watchdog (1.0.1 Batch 5).
//!
//! The single most expensive Fedora 1.0.0 failure produced NO TEXT: the
//! backend started clean, wrote its database, and the webview process aborted
//! — the app sat alive with no window, nothing on stderr, nothing in the log,
//! nothing in diagnostics (D1 holds zero panic/error rows from a day of
//! crashes, because a webview abort is not a Rust panic). This watchdog is
//! the "fail loudly" answer: if no page ever finishes loading, say so on
//! stderr AND in the log AND — when diagnostics are on — as a telemetry
//! `error` event the still-alive backend can flush.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Flipped by the Builder's `on_page_load` hook (PageLoadEvent::Finished).
pub static WEBVIEW_LOADED: AtomicBool = AtomicBool::new(false);

/// Generous: slow disks and cold starts are real; the failure this exists for
/// is INFINITE silence, not slowness. Cancel-on-first-paint means a loaded
/// machine that takes 29s emits nothing.
pub const TIMEOUT: Duration = Duration::from_secs(30);

/// The telemetry `error` class — greppable in D1.
pub const ERROR_CLASS: &str = "webview_launch_failed";

/// Arm the timer. A plain OS thread on purpose: setup runs on Tauri's main
/// thread outside any tokio context, and a watchdog for "the UI never came
/// up" must not depend on any other subsystem being healthy.
pub fn arm(local_dir: PathBuf) {
    std::thread::spawn(move || {
        std::thread::sleep(TIMEOUT);
        if WEBVIEW_LOADED.load(Ordering::Relaxed) {
            return;
        }
        let msg = format!(
            "bot-hq: the webview never finished loading within {}s — no window will \
             appear. The backend is running (this process stays alive); the webview \
             process likely crashed. On Linux AppImage installs see \
             docs/FEDORA-LINUX-COMPAT.md; capture stderr for EGL/library errors.",
            TIMEOUT.as_secs()
        );
        eprintln!("{msg}");
        tracing::error!("{msg}");
        if crate::core::telemetry::TELEMETRY_ENABLED.load(Ordering::Relaxed) {
            let ev = crate::core::telemetry::error_event(ERROR_CLASS, "startup_watchdog");
            if let Err(e) =
                crate::core::telemetry::enqueue(&crate::core::telemetry::queue_path(&local_dir), &ev)
            {
                tracing::warn!(?e, "watchdog telemetry enqueue failed");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    /// **The watchdog is WIRED, both ends** — the arm call and the
    /// cancel-on-first-paint flip both live in main.rs, where no test can
    /// execute them; pin their presence so neither end is droppable with a
    /// green suite (the exact defect class conventions.md's wire rule names).
    #[test]
    fn main_arms_the_watchdog_and_cancels_on_page_load() {
        // Uncommented lines only — a commented-out call must go red, not
        // slide past a bare `contains` (telemetry's guard had that hole).
        let live: String = include_str!("../main.rs")
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            live.contains("webview_watchdog::arm("),
            "main.rs must arm the startup watchdog (uncommented)"
        );
        assert!(
            live.contains("on_page_load"),
            "main.rs must hook page load — the cancel side"
        );
        assert!(
            live.contains("WEBVIEW_LOADED.store(true"),
            "the page-load hook must flip the flag the watchdog reads"
        );
    }
}
