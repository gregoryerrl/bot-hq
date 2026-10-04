//! `terminal_exec` / `terminal_read` MCP handlers — agents driving the
//! session's Terminal-subtab PTY (core/terminal.rs).
//!
//! `terminal_exec` is BLOCKING by default: it captures the scrollback offset,
//! types the command, then awaits output-settle (`wait_settle`) and returns
//! the captured output — the completion signal that keeps agents from racing
//! a fire-and-forget write with an immediate read. `block:false` opts out for
//! long-running processes (servers, watchers).
//!
//! Gate parity: the command is classified against the SAME two-tier Tool-Gate
//! keyword list the PreToolUse Bash hook enforces (`resolve_keywords` —
//! session snapshot first, global fallback). A `gate`-matched command is NOT
//! run; the agent is routed to `action_gate`, so the terminal can't serve as
//! a Tool-Gate bypass.

use super::SignalingBridge;
use crate::policy::tool_gate::{self, GateMode};
use anyhow::{anyhow, Result};

/// A command counts as finished once no output arrived for this long AND no
/// job held the terminal's foreground meanwhile — the shell is back at its
/// prompt (`SessionTerminal::wait_settle`, feedback #48). Where the platform
/// cannot report the foreground (Windows) the quiet window alone decides.
const EXEC_QUIET_MS: u64 = 700;
/// How long a first command waits for a freshly spawned shell's prompt.
const EXEC_READY_MAX_MS: u64 = 5_000;
/// Default / max total wait for the blocking exec.
const EXEC_DEFAULT_WAIT_MS: u64 = 10_000;
const EXEC_MAX_WAIT_MS: u64 = 120_000;
/// Output caps: exec returns at most this much of the tail; read defaults to
/// 100 lines and caps at 500.
const EXEC_OUTPUT_CAP_BYTES: usize = 16 * 1024;
const READ_DEFAULT_LINES: usize = 100;
const READ_MAX_LINES: usize = 500;

/// The tail of a command's output, at most [`EXEC_OUTPUT_CAP_BYTES`] of it —
/// that's where the result and the prompt are. A trimmed head is replaced by
/// a marker saying how much was cut.
fn capped_tail(output: String) -> String {
    if output.len() <= EXEC_OUTPUT_CAP_BYTES {
        return output;
    }
    // A char boundary at or before the byte offset: lossy decoding yields
    // valid UTF-8 but still multibyte glyphs (cargo's `━`, box-drawing `─`),
    // and a byte-offset slice inside one panics (round 9).
    let start = crate::text::ceil_char_boundary(&output, output.len() - EXEC_OUTPUT_CAP_BYTES);
    format!("[…{start} bytes trimmed…]\n{}", &output[start..])
}

/// The note a capped `terminal_exec` ends with. When a job still holds the
/// terminal it is named: a pager or a prompt waiting for input holds it to the
/// cap, and that is a different fix (`--no-pager`, a non-interactive flag)
/// from a long run (a larger `wait_ms`).
fn timeout_note(max_ms: u64, holder: Option<i32>) -> String {
    match holder {
        Some(group) => {
            let name = crate::core::terminal::process_name(group)
                .map(|n| format!("`{n}`"))
                .unwrap_or_else(|| format!("process group {group}"));
            format!(
                "\n[still running after {max_ms}ms — {name} holds the terminal (the shell is not \
                 back at its prompt). A pager or an interactive prompt waits for input until it \
                 gets some; a long run needs time: use terminal_read for later output, or \
                 terminal_exec with a larger wait_ms]"
            )
        }
        None => format!(
            "\n[still producing output after {max_ms}ms — command may be long-running; \
             use terminal_read for later output or terminal_exec with a larger wait_ms]"
        ),
    }
}

/// Programs whose input is a credential. Nothing is typed into them, even
/// with `to_job`: a command typed into `sudo` is a password attempt.
const CREDENTIAL_PROMPTS: &[&str] = &[
    "sudo", "su", "doas", "passwd", "login", "ssh", "ssh-add", "ssh-keygen", "gpg", "pinentry",
    "security", "kinit", "op",
];

/// Why `terminal_exec` will not type while `holder` — a job, not the shell —
/// holds the terminal (feedback #98): what is typed goes to that program's
/// input, so a pager, a server started with `block:false`, or a password
/// prompt the user left open would take the command. `to_job` says the agent
/// means to type into the job (a REPL it started); a credential prompt
/// anywhere in the job's pipeline (`group`) is refused even then.
fn busy_refusal(holder: &str, group: &[String], to_job: bool) -> Option<String> {
    if let Some(prompt) = std::iter::once(holder)
        .chain(group.iter().map(String::as_str))
        .find(|name| CREDENTIAL_PROMPTS.contains(name))
    {
        return Some(format!(
            "busy: `{prompt}` holds the terminal and may be waiting for a password — nothing is \
             typed into it, to_job or not. Answering or ending it is the user's."
        ));
    }
    (!to_job).then(|| {
        format!(
            "busy: `{holder}` holds the terminal (the shell is not at its prompt), so the command \
             would be typed into `{holder}`'s input. terminal_read shows what it is doing; wait \
             for it to finish. Pass to_job: true only to type a line into a program you started \
             there on purpose (a REPL)."
        )
    })
}

impl SignalingBridge {
    /// Gated on the `run_terminal` capability (enforced at the dispatch layer;
    /// any role may hold it). Runs `command` in the
    /// session's terminal and — unless `block` is false — returns the output
    /// captured until settle.
    pub async fn terminal_exec(
        &self,
        session_id: String,
        command: String,
        wait_ms: Option<u64>,
        block: Option<bool>,
        to_job: Option<bool>,
    ) -> Result<String> {
        let command = command.trim();
        if command.is_empty() {
            return Err(anyhow!("command must not be empty"));
        }
        if command.contains('\n') {
            // One command per call: a multiline payload would type stray
            // Enter presses into the shell (and could smuggle a second,
            // unclassified command past the gate check below).
            return Err(anyhow!(
                "multi-line commands are not supported — send one command per terminal_exec call"
            ));
        }

        // The zsh `$var:x` trap (feedback #96): the terminal runs the user's
        // login shell (`TerminalRegistry::default_shell`, `$SHELL`), so the
        // same check as the agent's own Bash, before anything is typed.
        let shell = std::env::var("SHELL").unwrap_or_default();
        if let Some(refusal) = crate::policy::shell_lint::zsh_trap(&shell, command) {
            return Err(anyhow!("{refusal}"));
        }

        // Group K: a command that runs one of the project's listed production
        // or staging reads is not typed here — it parks for the user's
        // approval, after the reviewer reads it.
        if let Some(read) = self.data_read_in(&session_id, command).await {
            return Err(anyhow!(
                "this runs a listed {} read (`{}` in the project's policy) — route it through \
                 action_gate: it parks for the user's approval, after the reviewer reads it",
                read.kind.label(),
                read.entry
            ));
        }

        // Tool-Gate parity (two-tier: session snapshot → global fallback).
        if let Some(d) = self.data_dir.as_ref() {
            let keywords = tool_gate::resolve_keywords(d, Some(&session_id));
            if tool_gate::match_keyword("Bash", command, &keywords) == Some(GateMode::Gate) {
                return Err(anyhow!(
                    "command matches a gated Tool-Gate keyword — route it through the \
                     action_gate tool instead (the terminal does not bypass the gate)"
                ));
            }
        }

        let registry = self
            .terminal_registry()
            .ok_or_else(|| anyhow!("terminal registry not initialized (app still starting?)"))?;

        // Same spawn inputs as the Terminal subtab's `terminal_open`: the
        // session's working repo (worktree-aware) as cwd, the app handle for
        // `terminal:output` emits so the user SEES agent-typed commands live.
        let storage = self.storage.lock().await.clone();
        let cwd = match storage {
            Some(storage) => storage
                .get_session(&session_id)
                .await?
                .and_then(|s| s.working_repo_path)
                .map(std::path::PathBuf::from),
            None => None,
        };
        let term = registry
            .ensure(&session_id, cwd, self.app_handle().cloned())
            .await?;
        // A shell not yet seen at its prompt — just spawned — may still be
        // reading its startup files with the typed line buffered: in the
        // foreground and silent, which the settle below would take for a
        // finished command. Paid once per terminal (`wait_ready`).
        if !term.is_ready() {
            term.wait_ready(EXEC_READY_MAX_MS).await;
        }

        // Feedback #98: typed text goes to whatever holds the terminal.
        if let crate::core::terminal::Foreground::Job(group) = term.foreground() {
            let holder = crate::core::terminal::process_name(group)
                .unwrap_or_else(|| format!("process group {group}"));
            let group_names = crate::core::terminal::group_process_names(group);
            if let Some(refusal) = busy_refusal(&holder, &group_names, to_job.unwrap_or(false)) {
                return Err(anyhow!(refusal));
            }
        }

        let offset = term.current_offset();
        // Windows consoles submit a line on CARRIAGE RETURN, not line feed.
        // cmd.exe echoes an LF-terminated write and then just sits there, so
        // the command never runs: observed live on this branch, where both
        // `git push -u origin windows-compat` and `ver` appeared at the prompt
        // and produced no output and no new prompt. That made `terminal_exec` —
        // a documented MCP tool — silently non-functional on Windows, and it
        // reads as a hung command rather than a broken one, which is why it was
        // mistaken for a credential prompt. Unix PTYs take `\n`.
        #[cfg(windows)]
        let submit = format!("{command}\r");
        #[cfg(not(windows))]
        let submit = format!("{command}\n");
        term.write_input(submit.as_bytes())?;

        if block == Some(false) {
            return Ok(format!(
                "command started (not waiting). Use terminal_read to inspect output later.\n$ {command}"
            ));
        }

        let max_ms = wait_ms
            .unwrap_or(EXEC_DEFAULT_WAIT_MS)
            .clamp(EXEC_QUIET_MS, EXEC_MAX_WAIT_MS);
        let settled = term.wait_settle(offset, EXEC_QUIET_MS, max_ms).await;
        let mut output = capped_tail(crate::core::term_text::render_plain(&settled.bytes));
        if settled.timed_out {
            output.push_str(&timeout_note(max_ms, settled.holder));
        }
        Ok(output)
    }

    /// Every participant (ungated). Tail of the terminal scrollback as lossy UTF-8 —
    /// evidence-grade text agents can paste into chat or IPAV docs. Reads a
    /// dead (exited) terminal's retained scrollback too.
    pub async fn terminal_read(&self, session_id: String, lines: Option<u64>, raw: Option<bool>) -> Result<String> {
        let registry = self
            .terminal_registry()
            .ok_or_else(|| anyhow!("terminal registry not initialized (app still starting?)"))?;
        let Some(term) = registry.get_any(&session_id).await else {
            return Ok("no terminal has been started for this session".to_string());
        };
        let (snapshot, _, _) = term.open_view();
        // As the screen shows it, unless the bytes themselves are wanted.
        let text = if raw == Some(true) {
            String::from_utf8_lossy(&snapshot).into_owned()
        } else {
            crate::core::term_text::render_plain(&snapshot)
        };
        let n = lines.unwrap_or(READ_DEFAULT_LINES as u64) as usize;
        let n = n.clamp(1, READ_MAX_LINES);
        let all: Vec<&str> = text.lines().collect();
        let tail = &all[all.len().saturating_sub(n)..];
        Ok(tail.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::{capped_tail, EXEC_OUTPUT_CAP_BYTES};
    use crate::core::TerminalRegistry;
    use crate::policy::tool_gate::{save, GateMode, GatedKeyword};
    use crate::policy::ViolationsLog;
    use crate::signaling::SignalingBridge;
    use std::sync::Arc;
    use tempfile::tempdir;

    #[tokio::test]
    async fn terminal_exec_refuses_gated_commands_without_spawning() {
        let dir = tempdir().unwrap();
        save(
            dir.path(),
            &[GatedKeyword {
                keyword: "push".into(),
                mode: GateMode::Gate,
            }],
        )
        .unwrap();
        let bridge =
            SignalingBridge::with_policy(ViolationsLog::new(dir.path()), dir.path().to_path_buf());
        let registry = Arc::new(TerminalRegistry::new());
        bridge.set_terminal_registry(Arc::clone(&registry));

        let err = bridge
            .terminal_exec("s1".into(), "git push origin main".into(), None, None, None)
            .await
            .expect_err("gated command must be refused");
        assert!(
            err.to_string().contains("action_gate"),
            "error should route to action_gate: {err}"
        );
        assert!(
            registry.get_any("s1").await.is_none(),
            "a refused command must not have spawned a terminal"
        );
    }

    /// Feedback #96: the terminal runs the user's `$SHELL`; under zsh a
    /// command carrying the `"$var:x"` trap is refused before anything is
    /// typed (or a terminal spawned). Skipped where `$SHELL` is not zsh.
    #[tokio::test]
    async fn terminal_exec_refuses_the_zsh_trap_without_spawning() {
        if !crate::policy::shell_lint::is_zsh(&std::env::var("SHELL").unwrap_or_default()) {
            eprintln!("$SHELL is not zsh here — nothing to refuse");
            return;
        }
        let dir = tempdir().unwrap();
        let bridge =
            SignalingBridge::with_policy(ViolationsLog::new(dir.path()), dir.path().to_path_buf());
        let registry = Arc::new(TerminalRegistry::new());
        bridge.set_terminal_registry(Arc::clone(&registry));
        let err = bridge
            .terminal_exec("s1".into(), r#"git show "$R:app/x""#.into(), None, None, None)
            .await
            .expect_err("the trap is refused");
        assert!(err.to_string().contains(r#"git show "${R}:app/x""#), "{err}");
        assert!(registry.get_any("s1").await.is_none(), "nothing was typed, nothing spawned");
    }

    /// Group K: a listed read is not typed into the terminal — it is routed
    /// to action_gate, which parks it after the reviewer's read.
    #[tokio::test]
    async fn terminal_exec_refuses_a_listed_read_without_spawning() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        std::fs::write(dir.path().join("config/general-policy.yaml"), "production_reads:\n  - bq\n").unwrap();
        let bridge =
            SignalingBridge::with_policy(ViolationsLog::new(dir.path()), dir.path().to_path_buf());
        let registry = Arc::new(TerminalRegistry::new());
        bridge.set_terminal_registry(Arc::clone(&registry));
        let err = bridge
            .terminal_exec("s1".into(), "bq ls --max_results 5".into(), None, None, None)
            .await
            .expect_err("a listed read is refused");
        assert!(err.to_string().contains("listed production read (`bq`"), "{err}");
        assert!(registry.get_any("s1").await.is_none(), "nothing typed, nothing spawned");
    }

    #[tokio::test]
    async fn terminal_exec_rejects_empty_and_multiline() {
        let bridge = SignalingBridge::new();
        bridge.set_terminal_registry(Arc::new(TerminalRegistry::new()));
        assert!(bridge
            .terminal_exec("s1".into(), "  ".into(), None, None, None)
            .await
            .is_err());
        assert!(bridge
            .terminal_exec("s1".into(), "echo a\necho b".into(), None, None, None)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn terminal_read_without_terminal_is_clean() {
        let bridge = SignalingBridge::new();
        bridge.set_terminal_registry(Arc::new(TerminalRegistry::new()));
        let out = bridge.terminal_read("s1".into(), None, None).await.unwrap();
        assert!(out.contains("no terminal"), "got: {out}");
    }

    /// Full blocking path through a REAL shell: exec captures settled output,
    /// then terminal_read sees the same scrollback. Uses the user's `$SHELL`
    /// (the production spawn path) — assertions stay loose on prompts/rc noise
    /// and only look for the echoed marker.
    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(windows, ignore = "ConPTY delivers no EOF, so the PTY death path never fires on Windows — deferred since 1.0.1 (CHANGELOG Deferred); measured failing on every main run")]
    async fn terminal_exec_blocking_then_read_round_trip() {
        let bridge = SignalingBridge::new();
        bridge.set_terminal_registry(Arc::new(TerminalRegistry::new()));
        // The marker is printed bold: both tools return it as the screen
        // shows it, without the escape bytes; `raw` keeps them.
        let out = bridge
            .terminal_exec(
                "s1".into(),
                r"printf '\033[1mbothq-exec-marker\033[0m\n'".into(),
                Some(15_000),
                None,
                None,
            )
            .await
            .expect("exec should succeed");
        assert!(
            out.contains("bothq-exec-marker") && !out.contains('\u{1b}'),
            "settled output missing marker, or not plain: {out:?}"
        );
        let read = bridge.terminal_read("s1".into(), None, None).await.unwrap();
        assert!(
            read.contains("bothq-exec-marker") && !read.contains('\u{1b}'),
            "terminal_read missing marker, or not plain: {read:?}"
        );
        let raw = bridge.terminal_read("s1".into(), None, Some(true)).await.unwrap();
        assert!(raw.contains("\u{1b}[1mbothq-exec-marker"), "raw keeps the bytes: {raw:?}");
    }

    /// Round 9: the tail cut used to be `&output[len - CAP..]` — a BYTE offset
    /// into lossy-decoded PTY output. cargo's progress bar (`━`, 3 bytes) and
    /// box-drawing (`─`) put a multibyte char across that offset routinely, and
    /// slicing inside one PANICS — inside `terminal_exec`, which surfaces as a
    /// missing tool result that reads as "the command hung". The cut must land on
    /// a char boundary.
    #[test]
    fn capped_tail_never_cuts_inside_a_multibyte_char() {
        // Enough 3-byte glyphs that SOME offset `len - CAP` lands mid-char for at
        // least one of the three alignments; assert all three.
        for pad in 0..3 {
            let mut s = "a".repeat(pad);
            s.push_str(&"━".repeat(EXEC_OUTPUT_CAP_BYTES)); // 3 × CAP bytes
            let out = capped_tail(s);
            assert!(out.starts_with("[…"), "marker missing: {}", &out[..40]);
            let body = out.split_once("…]\n").map(|(_, b)| b).unwrap();
            assert!(body.len() <= EXEC_OUTPUT_CAP_BYTES);
            assert!(body.chars().all(|c| c == '━'), "a partial glyph leaked into the tail");
        }
    }

    /// Feedback #98: a job holding the terminal is not typed into unless the
    /// agent says it means to (`to_job`), and a credential prompt never is.
    #[test]
    fn a_busy_terminal_is_refused_and_a_password_prompt_always() {
        let none: &[String] = &[];
        let refused = super::busy_refusal("less", none, false).expect("a pager takes no command");
        assert!(refused.starts_with("busy: `less` holds the terminal"), "{refused}");
        assert!(refused.contains("to_job: true"), "{refused}");
        assert_eq!(super::busy_refusal("python3", none, true), None, "a REPL the agent started");
        let sudo = super::busy_refusal("sudo", none, true).expect("never typed into");
        assert!(sudo.contains("password"), "{sudo}");
        // A password prompt behind the pipeline's leader (EYES, s-3158eb35).
        let piped = super::busy_refusal("cat", &["cat".into(), "sudo".into()], true).expect("refused");
        assert!(piped.starts_with("busy: `sudo` holds the terminal"), "{piped}");
    }

    /// Through a REAL shell: while `sleep` holds the terminal, a command is
    /// refused and names it; with `to_job` it is typed (into sleep's input).
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn terminal_exec_refuses_while_a_job_holds_the_terminal() {
        let registry = Arc::new(TerminalRegistry::new());
        let bridge = SignalingBridge::new();
        bridge.set_terminal_registry(registry.clone());
        bridge
            .terminal_exec("s1".into(), "sleep 5".into(), None, Some(false), None)
            .await
            .expect("started");
        let term = registry.get_live("s1").await.expect("a live terminal");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !matches!(term.foreground(), crate::core::terminal::Foreground::Job(_)) {
            assert!(std::time::Instant::now() < deadline, "sleep never took the terminal");
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        if let crate::core::terminal::Foreground::Job(group) = term.foreground() {
            let names = crate::core::terminal::group_process_names(group);
            assert!(names.iter().any(|n| n == "sleep"), "{names:?}");
        }
        let err = bridge
            .terminal_exec("s1".into(), "echo typed".into(), None, None, None)
            .await
            .expect_err("busy");
        assert!(err.to_string().starts_with("busy: `sleep` holds the terminal"), "{err}");
        bridge
            .terminal_exec("s1".into(), "echo typed".into(), None, Some(false), Some(true))
            .await
            .expect("typed on purpose");
        registry.kill_and_remove("s1").await;
    }

    /// A password prompt BEHIND a pipeline's leader is still seen: `sleep`
    /// leads, a program named `sudo` (a symlink to `sleep`, so nothing
    /// prompts; macOS kills a COPY of a system binary) follows, and even
    /// `to_job` is refused, naming `sudo`.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn terminal_exec_sees_a_credential_prompt_behind_the_leader() {
        let dir = tempdir().unwrap();
        let fake = dir.path().join("sudo");
        std::os::unix::fs::symlink("/bin/sleep", &fake).unwrap();
        let registry = Arc::new(TerminalRegistry::new());
        let bridge = SignalingBridge::new();
        bridge.set_terminal_registry(registry.clone());
        let pipeline = format!("sleep 5 | {} 5", fake.display());
        bridge.terminal_exec("s1".into(), pipeline, None, Some(false), None).await.expect("started");
        let term = registry.get_live("s1").await.expect("a live terminal");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let crate::core::terminal::Foreground::Job(group) = term.foreground() {
                if crate::core::terminal::group_process_names(group).iter().any(|n| n == "sudo") {
                    break;
                }
            }
            assert!(std::time::Instant::now() < deadline, "the pipeline never took the terminal");
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        let err = bridge
            .terminal_exec("s1".into(), "echo typed".into(), None, Some(false), Some(true))
            .await
            .expect_err("a password prompt is never typed into");
        assert!(err.to_string().starts_with("busy: `sudo` holds the terminal"), "{err}");
        registry.kill_and_remove("s1").await;
    }

    #[test]
    fn capped_tail_keeps_a_short_output_untouched() {
        let s = "cargo test ✓ 12 passed".to_string();
        assert_eq!(capped_tail(s.clone()), s);
    }
}
