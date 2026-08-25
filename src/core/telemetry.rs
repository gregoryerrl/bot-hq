//! Opt-in diagnostics (1.0.0 feature 2).
//!
//! Everything here is OFF until the user enables it (Settings → Diagnostics,
//! or the first-run card). What ships when enabled: `app_launch` (version, os,
//! arch), `panic` (sha256 hashes of the redacted message + backtrace — never
//! the text), and `error` (a short class + context tag from explicit call
//! sites). Never repo content, never prompts, never paths beyond a `$HOME`→`~`
//! redaction. The sink is the user-deployed Cloudflare worker in
//! `packaging/telemetry-worker/`; its URL is RUNTIME CONFIG (`app_settings`),
//! so there is no baked default endpoint — undeployed means idle, not dark
//! POSTs at a placeholder.
//!
//! Shape: events append to `<data_dir>/.local/telemetry.jsonl` (1 MB cap,
//! drop-oldest); a spawned flusher POSTs batches (launch + every 30 min) and
//! rewrites the file with the unsent remainder on success. Startup never
//! blocks on any of this. The panic hook is sync and DB-free: it reads the
//! [`TELEMETRY_ENABLED`] atomic (seeded at boot, flipped by the toggle
//! command) and appends directly.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

// ---- settings keys (app_settings) -----------------------------------------

pub const KEY_ENABLED: &str = "telemetry_enabled";
pub const KEY_INSTALL_ID: &str = "telemetry_install_id";
pub const KEY_ENDPOINT: &str = "telemetry_endpoint";
pub const KEY_ASKED: &str = "telemetry_asked";

/// Sync mirror of `telemetry_enabled` for the panic hook (which cannot await a
/// DB read mid-panic). Seeded at boot, flipped by the toggle command.
pub static TELEMETRY_ENABLED: AtomicBool = AtomicBool::new(false);

/// Queue cap: past this the OLDEST half is dropped. Diagnostics are droppable
/// by definition; the queue must never become the disk problem it reports on.
pub const QUEUE_CAP_BYTES: u64 = 1024 * 1024;
/// Max events per POST — mirrors the worker's `MAX_EVENTS`.
pub const BATCH_MAX: usize = 100;
/// Flusher period after the launch flush.
pub const FLUSH_EVERY: std::time::Duration = std::time::Duration::from_secs(30 * 60);

// ---- events ---------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QueuedEvent {
    pub kind: String,
    pub at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn app_launch_event() -> QueuedEvent {
    QueuedEvent {
        kind: "app_launch".into(),
        at: now_rfc3339(),
        data: Some(serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        })),
    }
}

pub fn panic_event(message: &str, backtrace: &str) -> QueuedEvent {
    let home = std::env::var("HOME").ok();
    QueuedEvent {
        kind: "panic".into(),
        at: now_rfc3339(),
        data: Some(serde_json::json!({
            "message_hash": sha256_hex(redact_home(message, home.as_deref()).as_bytes()),
            "backtrace_hash": sha256_hex(redact_home(backtrace, home.as_deref()).as_bytes()),
        })),
    }
}

pub fn error_event(class: &str, context_tag: &str) -> QueuedEvent {
    QueuedEvent {
        kind: "error".into(),
        at: now_rfc3339(),
        data: Some(serde_json::json!({
            "class": class,
            "context_tag": context_tag,
        })),
    }
}

/// `$HOME` → `~` wherever it appears. The one redaction rule: panic text and
/// backtraces routinely embed absolute paths, and the username inside them is
/// the identifying part.
pub fn redact_home(s: &str, home: Option<&str>) -> String {
    match home {
        Some(h) if !h.is_empty() => s.replace(h, "~"),
        _ => s.to_string(),
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

// ---- the on-disk queue ----------------------------------------------------

pub fn queue_path(local_dir: &Path) -> PathBuf {
    local_dir.join("telemetry.jsonl")
}

/// Append one event; enforce the cap by dropping the OLDEST half when crossed.
/// Sync + std-only so the panic hook can call it.
pub fn enqueue(path: &Path, ev: &QueuedEvent) -> Result<()> {
    use std::io::Write;
    let line = serde_json::to_string(ev).context("serializing telemetry event")?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    writeln!(f, "{line}").context("appending telemetry event")?;
    drop(f);
    let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if len > QUEUE_CAP_BYTES {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        let keep = &lines[lines.len() / 2..];
        std::fs::write(path, format!("{}\n", keep.join("\n")))
            .context("rewriting capped telemetry queue")?;
    }
    Ok(())
}

/// The whole queue, oldest first. Unparseable lines are dropped silently — a
/// torn tail from a crash mid-append must not wedge the flusher forever.
pub fn read_queue(path: &Path) -> Vec<QueuedEvent> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// Rewrite the queue to only the events NOT yet sent (everything after the
/// first `sent` entries).
pub fn drop_sent(path: &Path, sent: usize) -> Result<()> {
    let rest = read_queue(path).split_off(sent.min(read_queue(path).len()));
    if rest.is_empty() {
        let _ = std::fs::remove_file(path);
        return Ok(());
    }
    let body: String = rest
        .iter()
        .filter_map(|e| serde_json::to_string(e).ok())
        .map(|l| format!("{l}\n"))
        .collect();
    std::fs::write(path, body).context("rewriting telemetry queue after flush")
}

// ---- batch body (the worker's contract) -----------------------------------

/// The POST body `packaging/telemetry-worker/src/validate.ts` accepts. Pure,
/// so the shape is pinned by unit tests on THIS side of the wire too.
pub fn build_batch_body(install_id: &str, events: &[QueuedEvent]) -> String {
    serde_json::json!({
        "install_id": install_id,
        "app_version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "events": events,
    })
    .to_string()
}

// ---- flusher --------------------------------------------------------------

/// One flush attempt: enabled + endpoint + id + non-empty queue → POST up to
/// [`BATCH_MAX`] oldest events; drop them from the file only on 2xx. Errors
/// are logged at debug and swallowed — offline is a normal state.
pub async fn flush_once(storage: &crate::storage::Storage, local_dir: &Path) {
    if !TELEMETRY_ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let endpoint = match storage.get_setting(KEY_ENDPOINT).await {
        Ok(Some(e)) if !e.trim().is_empty() => e.trim().trim_end_matches('/').to_string(),
        _ => return,
    };
    let install_id = match storage.get_setting(KEY_INSTALL_ID).await {
        Ok(Some(id)) if !id.is_empty() => id,
        _ => return,
    };
    let path = queue_path(local_dir);
    let all = read_queue(&path);
    if all.is_empty() {
        return;
    }
    let batch: Vec<QueuedEvent> = all.iter().take(BATCH_MAX).cloned().collect();
    let body = build_batch_body(&install_id, &batch);
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => return,
    };
    let url = format!("{endpoint}/v1/events");
    match client
        .post(&url)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            let _ = drop_sent(&path, batch.len());
            tracing::debug!(sent = batch.len(), "telemetry flushed");
        }
        Ok(resp) => tracing::debug!(status = %resp.status(), "telemetry sink refused"),
        Err(e) => tracing::debug!(error = %e, "telemetry flush failed (offline is fine)"),
    }
}

/// Chain panic capture onto whatever hook is already installed (the child
/// reaper, in `main`): when the opt-in atomic is on, enqueue hashes of the
/// redacted panic text + backtrace, then hand off to the previous hook. Sync
/// and DB-free by construction — it runs mid-panic.
pub fn install_panic_capture(local_dir: &Path) {
    let path = queue_path(local_dir);
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if TELEMETRY_ENABLED.load(Ordering::Relaxed) {
            let bt = std::backtrace::Backtrace::force_capture().to_string();
            let _ = enqueue(&path, &panic_event(&info.to_string(), &bt));
        }
        prev(info);
    }));
}

/// Seed the enabled atomic from settings, enqueue this boot's `app_launch`
/// (when enabled), then flush on a spawned loop — first pass shortly after
/// launch, then every [`FLUSH_EVERY`]. Fire-and-forget from `main`.
pub fn start(storage: crate::storage::Storage, local_dir: PathBuf) {
    tokio::spawn(async move {
        let enabled = matches!(storage.get_setting(KEY_ENABLED).await, Ok(Some(v)) if v == "1");
        TELEMETRY_ENABLED.store(enabled, Ordering::Relaxed);
        if enabled {
            let _ = enqueue(&queue_path(&local_dir), &app_launch_event());
        }
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            flush_once(&storage, &local_dir).await;
            tokio::time::sleep(FLUSH_EVERY).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn redact_home_replaces_every_occurrence() {
        let s = "/Users/alice/x panicked at /Users/alice/y";
        assert_eq!(redact_home(s, Some("/Users/alice")), "~/x panicked at ~/y");
        assert_eq!(redact_home(s, None), s);
    }

    #[test]
    fn panic_event_carries_hashes_not_text() {
        let ev = panic_event("secret path /tmp/x", "frame one\nframe two");
        let data = ev.data.unwrap();
        let msg = data["message_hash"].as_str().unwrap();
        assert_eq!(msg.len(), 64);
        assert!(!serde_json::to_string(&data).unwrap().contains("secret"));
    }

    #[test]
    fn enqueue_appends_and_read_round_trips() {
        let d = tmp();
        let p = queue_path(d.path());
        enqueue(&p, &error_event("spawn", "agent_start")).unwrap();
        enqueue(&p, &app_launch_event()).unwrap();
        let q = read_queue(&p);
        assert_eq!(q.len(), 2);
        assert_eq!(q[0].kind, "error");
        assert_eq!(q[1].kind, "app_launch");
    }

    #[test]
    fn enqueue_caps_by_dropping_the_oldest_half() {
        let d = tmp();
        let p = queue_path(d.path());
        // ~1.6 KB per event via a fat context tag → cap crossed well before 1000.
        let fat = "x".repeat(1600);
        for i in 0..700 {
            enqueue(&p, &error_event(&format!("e{i}"), &fat)).unwrap();
        }
        let len = std::fs::metadata(&p).unwrap().len();
        assert!(len <= QUEUE_CAP_BYTES, "queue stayed capped (len {len})");
        let q = read_queue(&p);
        // The oldest events are the ones gone.
        assert_ne!(q[0].data.as_ref().unwrap()["class"], "e0");
    }

    #[test]
    fn drop_sent_keeps_the_unsent_tail_and_removes_an_emptied_file() {
        let d = tmp();
        let p = queue_path(d.path());
        for i in 0..3 {
            enqueue(&p, &error_event(&format!("e{i}"), "t")).unwrap();
        }
        drop_sent(&p, 2).unwrap();
        let q = read_queue(&p);
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].data.as_ref().unwrap()["class"], "e2");
        drop_sent(&p, 1).unwrap();
        assert!(!p.exists(), "fully-flushed queue file is removed");
    }

    #[test]
    fn torn_tail_lines_are_dropped_not_fatal() {
        let d = tmp();
        let p = queue_path(d.path());
        enqueue(&p, &error_event("ok", "t")).unwrap();
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        write!(f, "{{\"kind\":\"err").unwrap(); // crash mid-append
        drop(f);
        let q = read_queue(&p);
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn batch_body_matches_the_worker_contract() {
        let ev = error_event("spawn", "agent_start");
        let body = build_batch_body("9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d", &[ev]);
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["install_id"], "9b1deb4d-3b7d-4bad-9bdd-2b0d7b3dcb6d");
        assert_eq!(v["app_version"], env!("CARGO_PKG_VERSION"));
        assert!(v["os"].is_string() && v["arch"].is_string());
        let events = v["events"].as_array().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["kind"], "error");
        assert!(events[0]["at"].is_string());
        assert!(events[0]["data"].is_object());
    }
}
