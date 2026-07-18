//! Per-session PTY terminal backing the session view's Terminal subtab.
//!
//! One shell per session, spawned lazily in the session's working repo (the
//! worktree for isolated sessions — the same tree the agents mutate). Raw PTY
//! output lands in a bounded scrollback ring buffer with a monotonic byte
//! offset + a `Notify` on every append: the offset/notify pair is what lets
//! the (batch 5) blocking `terminal_exec` await output-settle instead of
//! racing a fire-and-forget write. Output is also coalesced (≤40 ms) into
//! `terminal:output` Tauri events for the xterm.js frontend.
//!
//! Terminals are in-memory only — no persistence, fresh per app run, killed
//! on `close_session` (mirrors the agent subprocess lifecycle).

use anyhow::{anyhow, Context, Result};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::{Mutex, Notify};

/// Scrollback byte cap. Old output evicts from the front; the absolute
/// offset keeps advancing so `since()` callers never see evicted bytes as
/// fresh ones.
pub const SCROLLBACK_CAP: usize = 200 * 1024;

/// Coalescing window for `terminal:output` emits — same spirit as the
/// message-path BatchEmitter (50 ms) without the per-session watermark.
const EMIT_FLUSH_MS: u64 = 40;

/// Default terminal geometry until the frontend's fit addon reports real
/// dimensions via `terminal_resize`.
const DEFAULT_COLS: u16 = 120;
const DEFAULT_ROWS: u16 = 30;

/// Tauri event name (no dots — Tauri 2 rejects them, see tauri_events/types).
pub const TERMINAL_OUTPUT_EVENT: &str = "terminal:output";

// ---------------------------------------------------------------------------
// Scrollback
// ---------------------------------------------------------------------------

/// Bounded byte ring with a monotonic absolute offset. `start` is the
/// absolute offset of `buf[0]`; `end_offset()` = `start + buf.len()` and only
/// ever grows, so a reader holding an offset can ask "everything since".
pub struct Scrollback {
    buf: Vec<u8>,
    start: u64,
    cap: usize,
}

impl Scrollback {
    pub fn new(cap: usize) -> Self {
        Self {
            buf: Vec::new(),
            start: 0,
            cap,
        }
    }

    pub fn append(&mut self, data: &[u8]) {
        // Oversized single chunk: keep only its tail — the front would evict
        // immediately anyway.
        if data.len() >= self.cap {
            self.start += (self.buf.len() + data.len() - self.cap) as u64;
            self.buf.clear();
            self.buf.extend_from_slice(&data[data.len() - self.cap..]);
            return;
        }
        let overflow = (self.buf.len() + data.len()).saturating_sub(self.cap);
        if overflow > 0 {
            self.buf.drain(..overflow);
            self.start += overflow as u64;
        }
        self.buf.extend_from_slice(data);
    }

    /// Absolute offset one past the last byte ever appended.
    pub fn end_offset(&self) -> u64 {
        self.start + self.buf.len() as u64
    }

    /// Everything currently retained (frontend replay on `terminal_open`).
    pub fn snapshot(&self) -> Vec<u8> {
        self.buf.clone()
    }

    /// Bytes appended at-or-after `offset`, clamped to what's still retained
    /// (an evicted range silently yields from the oldest retained byte).
    pub fn since(&self, offset: u64) -> Vec<u8> {
        let from = offset.saturating_sub(self.start).min(self.buf.len() as u64) as usize;
        self.buf[from..].to_vec()
    }
}

// ---------------------------------------------------------------------------
// SessionTerminal
// ---------------------------------------------------------------------------

/// One live PTY + shell. Cheap handles (`Arc`) are shared by the Tauri
/// command layer, the reader thread, and (batch 5) the MCP tool handlers.
pub struct SessionTerminal {
    pub session_id: String,
    scrollback: StdMutex<Scrollback>,
    /// Notified on every scrollback append AND on child exit — waiters must
    /// re-check state, not assume new bytes.
    output_notify: Notify,
    writer: StdMutex<Box<dyn Write + Send>>,
    master: StdMutex<Box<dyn MasterPty + Send>>,
    killer: StdMutex<Box<dyn ChildKiller + Send + Sync>>,
    cols: AtomicU16,
    rows: AtomicU16,
    /// Set by the reader thread on EOF (shell exited or was killed). A dead
    /// terminal is replaced on the next `ensure()`.
    dead: AtomicBool,
    emit_seq: AtomicU64,
}

impl SessionTerminal {
    /// Spawn `program` (the user's shell in production; `sh -c …` in tests)
    /// on a fresh PTY. `app` is the Tauri handle for `terminal:output`
    /// emits — `None` in unit tests keeps the terminal fully headless.
    pub fn spawn(
        session_id: &str,
        mut cmd: CommandBuilder,
        cwd: Option<PathBuf>,
        app: Option<tauri::AppHandle>,
    ) -> Result<Arc<Self>> {
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: DEFAULT_ROWS,
                cols: DEFAULT_COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("openpty failed")?;

        if let Some(dir) = cwd.filter(|d| d.is_dir()) {
            cmd.cwd(dir);
        }
        cmd.env("TERM", "xterm-256color");

        let child = pair
            .slave
            .spawn_command(cmd)
            .context("PTY shell spawn failed")?;
        let killer = child.clone_killer();
        // The slave fd stays with the child; dropping our copy is required or
        // reader EOF never fires after the shell exits.
        drop(pair.slave);

        let mut reader = pair
            .master
            .try_clone_reader()
            .context("clone PTY reader failed")?;
        let writer = pair.master.take_writer().context("take PTY writer failed")?;

        let term = Arc::new(Self {
            session_id: session_id.to_string(),
            scrollback: StdMutex::new(Scrollback::new(SCROLLBACK_CAP)),
            output_notify: Notify::new(),
            writer: StdMutex::new(writer),
            master: StdMutex::new(pair.master),
            killer: StdMutex::new(killer),
            cols: AtomicU16::new(DEFAULT_COLS),
            rows: AtomicU16::new(DEFAULT_ROWS),
            dead: AtomicBool::new(false),
            emit_seq: AtomicU64::new(0),
        });

        // Emit coalescer: reader thread pushes into `pending`, this task
        // drains every EMIT_FLUSH_MS while output flows. Skipped entirely in
        // headless (test) mode.
        let pending: Arc<StdMutex<Vec<u8>>> = Arc::new(StdMutex::new(Vec::new()));
        let emit_notify = Arc::new(Notify::new());
        if let Some(app) = app {
            let pending = Arc::clone(&pending);
            let emit_notify = Arc::clone(&emit_notify);
            let term = Arc::clone(&term);
            tokio::spawn(async move {
                use base64::Engine as _;
                use tauri::Emitter as _;
                loop {
                    emit_notify.notified().await;
                    // Coalesce whatever else lands inside the window.
                    tokio::time::sleep(std::time::Duration::from_millis(EMIT_FLUSH_MS)).await;
                    let chunk: Vec<u8> = std::mem::take(&mut *pending.lock().unwrap());
                    if chunk.is_empty() {
                        if term.dead.load(Ordering::Relaxed) {
                            break;
                        }
                        continue;
                    }
                    let payload = serde_json::json!({
                        "session_id": term.session_id,
                        "data": base64::engine::general_purpose::STANDARD.encode(&chunk),
                        "seq": term.emit_seq.fetch_add(1, Ordering::Relaxed),
                    });
                    if let Err(e) = app.emit(TERMINAL_OUTPUT_EVENT, payload) {
                        tracing::warn!(?e, "terminal:output emit failed");
                    }
                    if term.dead.load(Ordering::Relaxed) && pending.lock().unwrap().is_empty() {
                        break;
                    }
                }
            });
        }

        // Reader thread: blocking PTY reads — never on the tokio runtime.
        {
            let term = Arc::clone(&term);
            std::thread::Builder::new()
                .name(format!("pty-read-{}", &term.session_id[..8.min(term.session_id.len())]))
                .spawn(move || {
                    let mut buf = [0u8; 8192];
                    loop {
                        match reader.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                term.scrollback.lock().unwrap().append(&buf[..n]);
                                pending.lock().unwrap().extend_from_slice(&buf[..n]);
                                term.output_notify.notify_waiters();
                                emit_notify.notify_one();
                            }
                        }
                    }
                    let exit_note = b"\r\n[process exited]\r\n";
                    term.scrollback.lock().unwrap().append(exit_note);
                    pending.lock().unwrap().extend_from_slice(exit_note);
                    term.dead.store(true, Ordering::Relaxed);
                    term.output_notify.notify_waiters();
                    emit_notify.notify_one();
                })
                .context("spawn PTY reader thread failed")?;
        }

        Ok(term)
    }

    pub fn is_dead(&self) -> bool {
        self.dead.load(Ordering::Relaxed)
    }

    /// Snapshot + geometry for `terminal_open`'s replay.
    pub fn open_view(&self) -> (Vec<u8>, u16, u16) {
        (
            self.scrollback.lock().unwrap().snapshot(),
            self.cols.load(Ordering::Relaxed),
            self.rows.load(Ordering::Relaxed),
        )
    }

    /// Absolute end offset of everything written so far — capture BEFORE
    /// sending input, then `wait_settle(offset, …)` to collect the response.
    pub fn current_offset(&self) -> u64 {
        self.scrollback.lock().unwrap().end_offset()
    }

    pub fn write_input(&self, data: &[u8]) -> Result<()> {
        let mut w = self.writer.lock().unwrap();
        w.write_all(data).context("PTY write failed")?;
        w.flush().context("PTY flush failed")?;
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        if cols == 0 || rows == 0 {
            return Err(anyhow!("terminal size must be non-zero"));
        }
        self.master
            .lock()
            .unwrap()
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("PTY resize failed")?;
        self.cols.store(cols, Ordering::Relaxed);
        self.rows.store(rows, Ordering::Relaxed);
        Ok(())
    }

    pub fn kill(&self) {
        let _ = self.killer.lock().unwrap().kill();
    }

    /// Await output-settle: resolves once no new bytes have arrived for
    /// `quiet_ms` (or `max_ms` elapsed / the shell died), returning everything
    /// appended since `from_offset` plus whether the wait timed out. This is
    /// the completion signal behind the blocking `terminal_exec` — a quiet
    /// window heuristic, NOT prompt detection: interactive/TUI programs may
    /// settle early or ride to the cap.
    pub async fn wait_settle(&self, from_offset: u64, quiet_ms: u64, max_ms: u64) -> (Vec<u8>, bool) {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(max_ms);
        let quiet = std::time::Duration::from_millis(quiet_ms);
        let mut timed_out = false;
        loop {
            if self.is_dead() {
                break;
            }
            let seen = self.current_offset();
            let woke = tokio::select! {
                _ = self.output_notify.notified() => true,
                _ = tokio::time::sleep(quiet) => false,
            };
            if !woke && self.current_offset() == seen {
                break; // quiet window with no growth — settled
            }
            if tokio::time::Instant::now() >= deadline {
                timed_out = true;
                break;
            }
        }
        (
            self.scrollback.lock().unwrap().since(from_offset),
            timed_out,
        )
    }
}

// ---------------------------------------------------------------------------
// TerminalRegistry
// ---------------------------------------------------------------------------

/// Session-id → live terminal. One `Arc<TerminalRegistry>` hangs off
/// `AppState` (Tauri commands) and — from batch 5 — the `SignalingBridge`
/// (MCP tool handlers), so both layers reach the same PTY.
#[derive(Default)]
pub struct TerminalRegistry {
    terminals: Mutex<HashMap<String, Arc<SessionTerminal>>>,
}

impl TerminalRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// The user's login shell (`$SHELL`); the unset-var fallback is POSIX
    /// `/bin/sh` — guaranteed present, unlike zsh on minimal Linux.
    fn default_shell() -> CommandBuilder {
        #[cfg(windows)]
        {
            CommandBuilder::new("cmd.exe")
        }
        #[cfg(not(windows))]
        {
            CommandBuilder::new(std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()))
        }
    }

    /// Get the live terminal for a session, spawning (or replacing a dead)
    /// one on demand. `cwd` should be the session's working repo.
    pub async fn ensure(
        &self,
        session_id: &str,
        cwd: Option<PathBuf>,
        app: Option<tauri::AppHandle>,
    ) -> Result<Arc<SessionTerminal>> {
        let mut map = self.terminals.lock().await;
        if let Some(t) = map.get(session_id) {
            if !t.is_dead() {
                return Ok(Arc::clone(t));
            }
        }
        let term = SessionTerminal::spawn(session_id, Self::default_shell(), cwd, app)?;
        map.insert(session_id.to_string(), Arc::clone(&term));
        Ok(term)
    }

    /// The live terminal, if one is running (no spawn). Dead ones count as
    /// absent so callers don't write into an exited shell.
    pub async fn get_live(&self, session_id: &str) -> Option<Arc<SessionTerminal>> {
        self.terminals
            .lock()
            .await
            .get(session_id)
            .filter(|t| !t.is_dead())
            .map(Arc::clone)
    }

    /// Kill + drop a session's terminal (no-op when absent). `close_session`
    /// calls this alongside the agent-subprocess reaping.
    pub async fn kill_and_remove(&self, session_id: &str) {
        if let Some(t) = self.terminals.lock().await.remove(session_id) {
            t.kill();
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrollback_appends_and_reports_offsets() {
        let mut sb = Scrollback::new(10);
        sb.append(b"hello");
        assert_eq!(sb.end_offset(), 5);
        assert_eq!(sb.snapshot(), b"hello");
        assert_eq!(sb.since(2), b"llo");
        assert_eq!(sb.since(5), b"");
    }

    #[test]
    fn scrollback_evicts_front_and_keeps_offset_monotonic() {
        let mut sb = Scrollback::new(10);
        sb.append(b"0123456789");
        sb.append(b"abc"); // evicts "012"
        assert_eq!(sb.end_offset(), 13);
        assert_eq!(sb.snapshot(), b"3456789abc");
        // Asking for an evicted range clamps to the oldest retained byte.
        assert_eq!(sb.since(0), b"3456789abc");
        assert_eq!(sb.since(11), b"bc");
    }

    #[test]
    fn scrollback_oversized_chunk_keeps_tail() {
        let mut sb = Scrollback::new(4);
        sb.append(b"abcdefgh");
        assert_eq!(sb.snapshot(), b"efgh");
        assert_eq!(sb.end_offset(), 8);
        assert_eq!(sb.since(0), b"efgh");
    }

    /// Spawn helper for PTY tests: a real shell running one command.
    fn spawn_sh(script: &str) -> Arc<SessionTerminal> {
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.arg("-c");
        cmd.arg(script);
        SessionTerminal::spawn("test-session", cmd, None, None).expect("pty spawn")
    }

    async fn wait_for<F: Fn(&SessionTerminal) -> bool>(
        term: &SessionTerminal,
        pred: F,
        ms: u64,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(ms);
        while tokio::time::Instant::now() < deadline {
            if pred(term) {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        pred(term)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pty_round_trip_captures_output() {
        let term = spawn_sh("printf 'hello-from-pty'");
        let ok = wait_for(
            &term,
            |t| {
                String::from_utf8_lossy(&t.scrollback.lock().unwrap().snapshot())
                    .contains("hello-from-pty")
            },
            5_000,
        )
        .await;
        assert!(ok, "PTY output never contained the marker");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pty_reader_marks_dead_on_exit() {
        let term = spawn_sh("exit 0");
        assert!(
            wait_for(&term, |t| t.is_dead(), 5_000).await,
            "terminal never marked dead after shell exit"
        );
        let snap = String::from_utf8_lossy(&term.scrollback.lock().unwrap().snapshot()).to_string();
        assert!(snap.contains("[process exited]"), "missing exit note: {snap:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn kill_terminates_long_running_shell() {
        let term = spawn_sh("sleep 30");
        term.kill();
        assert!(
            wait_for(&term, |t| t.is_dead(), 5_000).await,
            "kill did not end the PTY session"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn wait_settle_returns_output_since_offset() {
        let term = spawn_sh("printf 'before'; sleep 0.2; printf 'after-marker'; sleep 30");
        // Let the first chunk land, then capture the offset and wait for the
        // rest to settle.
        assert!(
            wait_for(
                &term,
                |t| t.current_offset() >= "before".len() as u64,
                5_000
            )
            .await
        );
        let offset = term.current_offset();
        let (out, timed_out) = term.wait_settle(offset, 600, 8_000).await;
        let text = String::from_utf8_lossy(&out);
        assert!(!timed_out, "settle wait should not time out");
        assert!(
            text.contains("after-marker"),
            "settled output missing marker: {text:?}"
        );
        assert!(
            !text.contains("before"),
            "offset-scoped read leaked earlier output: {text:?}"
        );
        term.kill();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn wait_settle_times_out_on_endless_output() {
        let term = spawn_sh("while true; do printf x; sleep 0.05; done");
        let (out, timed_out) = term.wait_settle(0, 400, 1_200).await;
        assert!(timed_out, "endless output must hit the max_ms cap");
        assert!(!out.is_empty());
        term.kill();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn registry_replaces_dead_terminal_and_kills_on_remove() {
        let reg = TerminalRegistry::new();
        // ensure() with the default shell would open a real login shell; use
        // the spawn helper via the map directly to keep the test hermetic.
        let dead = spawn_sh("exit 0");
        assert!(wait_for(&dead, |t| t.is_dead(), 5_000).await);
        reg.terminals
            .lock()
            .await
            .insert("s1".into(), Arc::clone(&dead));
        assert!(reg.get_live("s1").await.is_none(), "dead terminal leaked as live");

        let live = spawn_sh("sleep 30");
        reg.terminals
            .lock()
            .await
            .insert("s1".into(), Arc::clone(&live));
        assert!(reg.get_live("s1").await.is_some());
        reg.kill_and_remove("s1").await;
        assert!(wait_for(&live, |t| t.is_dead(), 5_000).await);
        assert!(reg.get_live("s1").await.is_none());
    }
}
