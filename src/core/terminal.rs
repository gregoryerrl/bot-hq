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
    /// The shell's pid. It leads its own session and process group (the PTY
    /// spawn makes it a session leader), so "the shell is in the foreground"
    /// is `tcgetpgrp(master) == shell_pid` — see [`Self::foreground`].
    shell_pid: Option<u32>,
    /// The shell has been seen at its first prompt ([`Self::wait_ready`]).
    ready: AtomicBool,
}

/// Who holds a terminal's foreground: the shell itself (at its prompt, or
/// running a builtin), a job it started, or — where the platform cannot say
/// (Windows' ConPTY, a failed read) — unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Foreground {
    Shell,
    /// The foreground process group's id.
    Job(i32),
    Unknown,
}

/// What [`SessionTerminal::wait_settle`] saw.
#[derive(Debug)]
pub struct Settled {
    /// Everything appended since the offset the wait was given.
    pub bytes: Vec<u8>,
    /// The `max_ms` cap ended the wait.
    pub timed_out: bool,
    /// On a timeout, the foreground job still holding the terminal — the
    /// command that has not given the prompt back (a long run, a pager, a
    /// prompt waiting for input). `None` when the shell held it, or when the
    /// platform cannot say.
    pub holder: Option<i32>,
}

/// How often a settle wait samples the foreground and the output, between
/// output notifications.
const SETTLE_TICK: std::time::Duration = std::time::Duration::from_millis(25);
/// The quiet that counts as "the first prompt has been printed".
const READY_QUIET: std::time::Duration = std::time::Duration::from_millis(300);

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
        // Under an AppImage launch this PTY inherits the payload's library
        // paths, and the user WATCHES this one: the leak surfaces as
        // `flatpak: symbol lookup error` and libpcre2 warnings on every shell
        // startup. A no-op off a payload. See `appimage_env`.
        crate::appimage_env::scrub_pty(&mut cmd);
        cmd.env("TERM", "xterm-256color");
        // The git hooks read BOT_HQ_SESSION_ID to know whose session a commit
        // or push belongs to (`policy/hooks.rs::hook_session_id`): without it
        // the blocking-findings gate returns "gate N/A" and a push under
        // `push_gate = ask` fails closed. Only the agent subprocess carried it
        // (`spawn.rs::build_command`), so a `git commit` typed through
        // `terminal_exec` bypassed the findings gate the descriptors promise
        // and a `git push` there was refused with "no bot-hq session context"
        // (round 8, M1; CODEBASE.md seam 6). The PTY is the session's shell —
        // whoever types in it, agent or user, commits and pushes as that
        // session, which is what the gates are scoped to.
        cmd.env("BOT_HQ_SESSION_ID", session_id);

        let child = pair
            .slave
            .spawn_command(cmd)
            .context("PTY shell spawn failed")?;
        let killer = child.clone_killer();
        let shell_pid = child.process_id();
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
            shell_pid,
            ready: AtomicBool::new(false),
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
                    // The child rides into this thread so it can be REAPED
                    // here. `portable_pty`'s unix child is a
                    // `std::process::Child`, which does not wait on drop, and
                    // `kill()` only signals — so every terminal that ever
                    // exited left a zombie behind for the life of the app.
                    // Read-EOF is exactly the moment the process is gone, so
                    // the wait below returns immediately; it is a reap, not a
                    // block.
                    let mut child = child;
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
                    // Reap before announcing the exit: after this the process
                    // is really gone rather than a zombie in the table.
                    if let Err(e) = child.wait() {
                        tracing::warn!(?e, "PTY child wait failed; it may be left a zombie");
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

    /// Signal the shell. The REAP is the reader thread's, at read-EOF — which
    /// the kill causes, so the two are one sequence: signal here, the read ends,
    /// the thread waits the child and announces the exit.
    pub fn kill(&self) {
        let _ = self.killer.lock().unwrap().kill();
    }

    /// Who holds the terminal's foreground right now (unix: the master's
    /// `tcgetpgrp`, compared with the shell's pid — probed on macOS,
    /// s-3158eb35: the shell's pid at the prompt, the job's group while it
    /// runs, the shell's again after). Windows' ConPTY has no such call.
    pub fn foreground(&self) -> Foreground {
        #[cfg(unix)]
        {
            let leader = self.master.lock().unwrap().process_group_leader();
            match (self.shell_pid, leader) {
                (Some(shell), Some(group)) if i64::from(shell) == i64::from(group) => Foreground::Shell,
                (Some(_), Some(group)) => Foreground::Job(group),
                _ => Foreground::Unknown,
            }
        }
        #[cfg(not(unix))]
        {
            Foreground::Unknown
        }
    }

    /// Await the command's end (feedback #48): resolves once `quiet_ms` pass
    /// with no new output AND no job in the foreground — or at `max_ms`, or
    /// when the shell dies — returning everything appended since
    /// `from_offset`. This is the completion signal behind the blocking
    /// `terminal_exec`.
    ///
    /// Quiet alone was the old rule, and a command with a quiet start ended
    /// it early: `git … && npm test | tail` returned after the git lines,
    /// while vitest was still starting (#48). A job holding the terminal now
    /// restarts the quiet clock, and the foreground is sampled every
    /// [`SETTLE_TICK`] THROUGH the window rather than once at its end, because
    /// the shell takes the terminal back for a few milliseconds between the
    /// steps of an `&&` chain (EYES, s-3158eb35). Builtins never leave the
    /// shell, so they settle on quiet as before; where the foreground is
    /// unknown (Windows) the rule is the old quiet window exactly.
    ///
    /// A command that waits for input — a pager, a prompt — holds the
    /// foreground until `max_ms`; [`Settled::holder`] names it then.
    pub async fn wait_settle(&self, from_offset: u64, quiet_ms: u64, max_ms: u64) -> Settled {
        let start = tokio::time::Instant::now();
        let deadline = start + std::time::Duration::from_millis(max_ms);
        let quiet = std::time::Duration::from_millis(quiet_ms);
        let mut seen = self.current_offset();
        let mut quiet_since = start;
        let mut holder = None;
        let mut timed_out = false;
        loop {
            if self.is_dead() {
                break;
            }
            tokio::select! {
                _ = self.output_notify.notified() => {}
                _ = tokio::time::sleep(SETTLE_TICK) => {}
            }
            let now = tokio::time::Instant::now();
            let offset = self.current_offset();
            if offset != seen {
                seen = offset;
                quiet_since = now;
            }
            holder = match self.foreground() {
                Foreground::Job(group) => {
                    quiet_since = now;
                    Some(group)
                }
                Foreground::Shell | Foreground::Unknown => None,
            };
            if now.duration_since(quiet_since) >= quiet {
                break; // quiet, and the shell holds the terminal — settled
            }
            if now >= deadline {
                timed_out = true;
                break;
            }
        }
        Settled {
            bytes: self.scrollback.lock().unwrap().since(from_offset),
            timed_out,
            holder: if timed_out { holder } else { None },
        }
    }

    /// The shell has been seen at its first prompt.
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }

    /// Wait, at most `max_ms`, for a freshly spawned shell to print its first
    /// prompt: some output, then [`READY_QUIET`] with no job in the
    /// foreground. Until then a typed line sits in the buffer while the shell
    /// reads its startup files — in the foreground and silent, which a settle
    /// wait would take for a finished command. Returns whether the prompt was
    /// seen; the terminal counts as ready either way, so the wait is paid once.
    ///
    /// Known race: a shell with an "instant prompt" (powerlevel10k) prints
    /// before its startup files finish. The worst case is the old early
    /// return, on the first command into that shell only.
    pub async fn wait_ready(&self, max_ms: u64) -> bool {
        let start = tokio::time::Instant::now();
        let deadline = start + std::time::Duration::from_millis(max_ms);
        let mut seen = self.current_offset();
        let mut quiet_since = start;
        let seen_prompt = loop {
            if self.is_dead() {
                break false;
            }
            tokio::select! {
                _ = self.output_notify.notified() => {}
                _ = tokio::time::sleep(SETTLE_TICK) => {}
            }
            let now = tokio::time::Instant::now();
            let offset = self.current_offset();
            if offset != seen {
                seen = offset;
                quiet_since = now;
            }
            if matches!(self.foreground(), Foreground::Job(_)) {
                quiet_since = now;
            }
            if offset > 0 && now.duration_since(quiet_since) >= READY_QUIET {
                break true;
            }
            if now >= deadline {
                break false;
            }
        };
        self.ready.store(true, Ordering::Relaxed);
        seen_prompt
    }
}

/// The name of process `pid` (`ps -o comm=`, the last path segment), for the
/// note that says which command holds a terminal. `None` where it cannot be
/// read.
/// The basenames of every process in process group `pgid` — the whole
/// foreground pipeline, not only its leader (`cat x | sudo tee f` is led by
/// `cat` while `sudo` waits for a password; EYES, s-3158eb35). One `ps -A`
/// in the form macOS and Linux share, filtered here. Empty off unix or when
/// `ps` fails.
pub fn group_process_names(pgid: i32) -> Vec<String> {
    #[cfg(unix)]
    {
        let Ok(out) = std::process::Command::new("ps").args(["-A", "-o", "pgid=,comm="]).output() else {
            return Vec::new();
        };
        if !out.status.success() {
            return Vec::new();
        }
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|line| {
                let line = line.trim_start();
                let (group, name) = line.split_once(char::is_whitespace)?;
                (group.trim().parse::<i32>().ok()? == pgid).then(|| {
                    let name = name.trim();
                    name.rsplit('/').next().unwrap_or(name).to_string()
                })
            })
            .filter(|n| !n.is_empty())
            .collect()
    }
    #[cfg(not(unix))]
    {
        let _ = pgid;
        Vec::new()
    }
}

pub fn process_name(pid: i32) -> Option<String> {
    #[cfg(unix)]
    {
        let out = std::process::Command::new("ps")
            .args(["-o", "comm=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let base = name.rsplit('/').next().unwrap_or(&name).trim().to_string();
        (out.status.success() && !base.is_empty()).then_some(base)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        None
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

    /// The session's terminal live OR dead — an exited shell's retained
    /// scrollback is still readable evidence (`terminal_read`).
    pub async fn get_any(&self, session_id: &str) -> Option<Arc<SessionTerminal>> {
        self.terminals.lock().await.get(session_id).map(Arc::clone)
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

    /// **The PTY child is REAPED, not just killed** (B2-6).
    ///
    /// `portable_pty`'s unix child is a `std::process::Child`: it does not wait
    /// on drop, and `kill()` only signals. The child was dropped at the end of
    /// `spawn`, so every terminal that ever exited left a zombie in the process
    /// table for the life of the app — one per terminal, invisible until you
    /// look.
    ///
    /// Asserted over the source: the reap happens on a blocking reader THREAD
    /// after read-EOF, and a test that wanted to observe it would have to spawn
    /// a real shell and then inspect the process table, which is neither
    /// portable nor deterministic. What is checkable is that the child reaches
    /// the thread and is waited there — the two things that were missing.
    #[test]
    fn the_pty_child_is_waited_not_just_dropped() {
        let src = include_str!("terminal.rs");
        let prod = src
            .split("mod tests {")
            .next()
            .expect("a split always yields a first part");
        let thread = prod
            .split("Reader thread: blocking PTY reads")
            .nth(1)
            .expect("the reader thread exists");
        assert!(
            thread.contains("let mut child = child;"),
            "the child must ride into the reader thread — that is where read-EOF \
             says the process is gone"
        );
        assert!(
            thread.contains("child.wait()"),
            "a killed child that is never waited is a zombie, one per terminal"
        );
    }

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
    ///
    /// Uses the SAME shell production uses on each platform (see
    /// [`default_shell`]): `cmd.exe` on Windows, `/bin/sh` elsewhere.
    ///
    /// Deliberately NOT the MSYS `sh` that `policy::tool_gate::posix_shell`
    /// resolves for gated commands. The terminal never spawns that here, and
    /// MSYS-built programs are exactly the class whose ConPTY behaviour
    /// diverges — it is why `winpty` exists. Greening these tests under a shell
    /// the product does not use would be a coverage illusion, not coverage.
    ///
    /// The scripts are scaffolding; the PTY mechanics are the assertion. Every
    /// assertion here is `contains`-based, so cmd.exe's CRLF line terminator
    /// (it has no `printf`, and `echo` always terminates the line) does not
    /// change what any of them test.
    fn spawn_shell(script: &str) -> Arc<SessionTerminal> {
        #[cfg(windows)]
        let mut cmd = {
            let mut c = CommandBuilder::new("cmd.exe");
            c.arg("/C");
            c
        };
        #[cfg(not(windows))]
        let mut cmd = {
            let mut c = CommandBuilder::new("/bin/sh");
            c.arg("-c");
            c
        };
        cmd.arg(script);
        let term = SessionTerminal::spawn("test-session", cmd, None, None).expect("pty spawn");
        // ConPTY opens by emitting a DSR cursor-position query (`ESC[6n`) and
        // waits for the host to answer `ESC[<row>;<col>R` before the child's
        // output flows. In the app xterm.js answers automatically, which is why
        // production works and every headless PTY test saw a scrollback of
        // exactly "\u{1b}[6n" and nothing else — long-lived children included,
        // which is what ruled out a teardown race.
        //
        // This is a HARNESS gap, not a product defect: the product's PTY path
        // is fine (the session Terminal renders cmd.exe's banner live). Answer
        // the query once so the tests have the responder the app normally
        // supplies.
        //
        // UNCONDITIONAL IS CORRECT **HERE AND ONLY HERE**. These children are
        // `cmd.exe /C <script>`, which never reads stdin, so an early or
        // duplicate response is harmless. Do NOT promote this into
        // `SessionTerminal` as-is: writing the response unprompted races the
        // query, and in an INTERACTIVE shell a stray `ESC[1;1R` arrives as
        // typed characters sitting on the command line. A production responder
        // has to be REACTIVE — trigger on observing `ESC[6n` in the output
        // stream, in the reader loop.
        #[cfg(windows)]
        let _ = term.write_input(b"\x1b[1;1R");
        term
    }

    /// The PTY test scripts: same intent per platform, native syntax.
    ///
    /// cmd.exe has no `printf` and no `sleep`. `ping -n <N+1> 127.0.0.1` is the
    /// robust "block for N seconds" idiom for a `/C` one-liner — deliberately
    /// NOT `timeout /t`, which refuses to run when stdin is redirected. `&`
    /// separates commands and is written unspaced, since `echo before& …` would
    /// otherwise emit a trailing space.
    mod script {
        /// Echo the session id the git hooks read.
        #[cfg(windows)]
        pub const ECHO_SESSION_ID: &str = "echo sid=%BOT_HQ_SESSION_ID%";
        #[cfg(not(windows))]
        pub const ECHO_SESSION_ID: &str = "printf 'sid=%s' \"$BOT_HQ_SESSION_ID\"";

        /// Emit one marker and exit.
        #[cfg(windows)]
        pub const ECHO_MARKER: &str = "echo hello-from-pty";
        #[cfg(not(windows))]
        pub const ECHO_MARKER: &str = "printf 'hello-from-pty'";

        /// Exit immediately, cleanly.
        pub const EXIT_OK: &str = "exit 0";

        /// Stay alive well past any test's timeout.
        #[cfg(windows)]
        pub const STAY_ALIVE: &str = "ping -n 31 127.0.0.1 >nul";
        #[cfg(not(windows))]
        pub const STAY_ALIVE: &str = "sleep 30";

        /// Two output chunks separated by a gap, then stay alive — so a reader
        /// can capture an offset between them.
        #[cfg(windows)]
        pub const TWO_CHUNKS_THEN_ALIVE: &str =
            "echo before&ping -n 2 127.0.0.1 >nul&echo after-marker&ping -n 31 127.0.0.1 >nul";
        #[cfg(not(windows))]
        pub const TWO_CHUNKS_THEN_ALIVE: &str =
            "printf 'before'; sleep 0.2; printf 'after-marker'; sleep 30";

        /// Never stop emitting. Must produce output more often than the
        /// settle window (400ms), so the `ping` idiom is wrong here — its 1s
        /// granularity would let the reader settle between chunks. A zero-step
        /// `for /L` counter never terminates.
        #[cfg(windows)]
        pub const ENDLESS_OUTPUT: &str = "for /l %i in (1,0,2) do @echo x";
        #[cfg(not(windows))]
        pub const ENDLESS_OUTPUT: &str = "while true; do printf x; sleep 0.05; done";
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

    /// **The PTY carries the session id the git hooks read** (round 8, M1).
    /// Without it a `git commit` typed through `terminal_exec` skipped the
    /// blocking-findings gate ("no session context → gate N/A") and a push
    /// under `push_gate = ask` failed closed. Kill-tested: drop the
    /// `cmd.env("BOT_HQ_SESSION_ID", …)` in `spawn` and this goes red.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_pty_carries_the_session_id_the_hooks_read() {
        let term = spawn_shell(script::ECHO_SESSION_ID);
        let ok = wait_for(
            &term,
            |t| {
                String::from_utf8_lossy(&t.scrollback.lock().unwrap().snapshot())
                    .contains("sid=test-session")
            },
            5_000,
        )
        .await;
        assert!(
            ok,
            "the shell did not see BOT_HQ_SESSION_ID=test-session. scrollback={:?}",
            String::from_utf8_lossy(&term.scrollback.lock().unwrap().snapshot())
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pty_round_trip_captures_output() {
        let term = spawn_shell(script::ECHO_MARKER);
        let ok = wait_for(
            &term,
            |t| {
                String::from_utf8_lossy(&t.scrollback.lock().unwrap().snapshot())
                    .contains("hello-from-pty")
            },
            5_000,
        )
        .await;
        assert!(
            ok,
            "PTY output never contained the marker. scrollback={:?}",
            String::from_utf8_lossy(&term.scrollback.lock().unwrap().snapshot())
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(windows, ignore = "ConPTY delivers no EOF, so the PTY death path never fires on Windows — deferred since 1.0.1 (CHANGELOG Deferred); measured failing on every main run")]
    async fn pty_reader_marks_dead_on_exit() {
        let term = spawn_shell(script::EXIT_OK);
        assert!(
            wait_for(&term, |t| t.is_dead(), 5_000).await,
            "terminal never marked dead after shell exit"
        );
        let snap = String::from_utf8_lossy(&term.scrollback.lock().unwrap().snapshot()).to_string();
        assert!(snap.contains("[process exited]"), "missing exit note: {snap:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(windows, ignore = "ConPTY delivers no EOF, so the PTY death path never fires on Windows — deferred since 1.0.1 (CHANGELOG Deferred); measured failing on every main run")]
    async fn kill_terminates_long_running_shell() {
        let term = spawn_shell(script::STAY_ALIVE);
        term.kill();
        assert!(
            wait_for(&term, |t| t.is_dead(), 5_000).await,
            "kill did not end the PTY session"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(windows, ignore = "ConPTY delivers no EOF, so the PTY death path never fires on Windows — deferred since 1.0.1 (CHANGELOG Deferred); measured failing on every main run")]
    async fn wait_settle_returns_output_since_offset() {
        let term = spawn_shell(script::TWO_CHUNKS_THEN_ALIVE);
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
        let settled = term.wait_settle(offset, 600, 8_000).await;
        let (out, timed_out) = (settled.bytes, settled.timed_out);
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
        let term = spawn_shell(script::ENDLESS_OUTPUT);
        let settled = term.wait_settle(0, 400, 1_200).await;
        let (out, timed_out) = (settled.bytes, settled.timed_out);
        assert!(timed_out, "endless output must hit the max_ms cap");
        assert!(!out.is_empty());
        term.kill();
    }

    /// An interactive POSIX shell, so commands run as jobs in their own
    /// process group — the terminal the agents type into is one.
    #[cfg(unix)]
    fn spawn_interactive_sh() -> Arc<SessionTerminal> {
        let mut cmd = CommandBuilder::new("/bin/sh");
        cmd.arg("-i");
        cmd.env("PS1", "$ ");
        cmd.env("BASH_SILENCE_DEPRECATION_WARNING", "1");
        SessionTerminal::spawn("test-session", cmd, None, None).expect("pty spawn")
    }

    /// Feedback #48: a command that runs quietly for longer than the quiet
    /// window is waited for while it holds the terminal — one wait captures
    /// the whole run. Under the old quiet-only rule this returned the echo of
    /// the typed line and nothing else.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_quiet_job_is_waited_for_until_the_prompt_returns() {
        let term = spawn_interactive_sh();
        assert!(term.wait_ready(5_000).await, "the shell printed its prompt");
        let offset = term.current_offset();
        // `DO''NE`: the echo of the typed line must not satisfy the check.
        term.write_input(b"sleep 1.5 && echo DO''NE\n").unwrap();
        let settled = term.wait_settle(offset, 700, 10_000).await;
        let text = String::from_utf8_lossy(&settled.bytes).to_string();
        assert!(!settled.timed_out, "{text:?}");
        assert!(text.contains("DONE"), "one wait captures the whole run: {text:?}");
        term.kill();
    }

    /// A builtin never leaves the shell, so it settles on quiet, as before.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_builtin_settles_on_quiet_as_before() {
        let term = spawn_interactive_sh();
        assert!(term.wait_ready(5_000).await);
        let offset = term.current_offset();
        let started = std::time::Instant::now();
        term.write_input(b"echo built''in\n").unwrap();
        let settled = term.wait_settle(offset, 700, 10_000).await;
        assert!(!settled.timed_out);
        assert!(String::from_utf8_lossy(&settled.bytes).contains("builtin"));
        assert!(started.elapsed() < std::time::Duration::from_secs(3), "{:?}", started.elapsed());
        term.kill();
    }

    /// A job still holding the terminal at the cap — a long run, a pager, a
    /// prompt waiting for input — is named, so the note can say which.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_job_still_holding_the_terminal_at_the_cap_is_named() {
        let term = spawn_interactive_sh();
        assert!(term.wait_ready(5_000).await);
        let offset = term.current_offset();
        term.write_input(b"sleep 5\n").unwrap();
        let settled = term.wait_settle(offset, 700, 1_500).await;
        assert!(settled.timed_out);
        let holder = settled.holder.expect("a job holds the terminal");
        assert_eq!(process_name(holder).as_deref(), Some("sleep"));
        term.kill();
    }

    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(windows, ignore = "ConPTY delivers no EOF, so the PTY death path never fires on Windows — deferred since 1.0.1 (CHANGELOG Deferred); measured failing on every main run")]
    async fn registry_replaces_dead_terminal_and_kills_on_remove() {
        let reg = TerminalRegistry::new();
        // ensure() with the default shell would open a real login shell; use
        // the spawn helper via the map directly to keep the test hermetic.
        let dead = spawn_shell(script::EXIT_OK);
        assert!(wait_for(&dead, |t| t.is_dead(), 5_000).await);
        reg.terminals
            .lock()
            .await
            .insert("s1".into(), Arc::clone(&dead));
        assert!(reg.get_live("s1").await.is_none(), "dead terminal leaked as live");

        let live = spawn_shell(script::STAY_ALIVE);
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
