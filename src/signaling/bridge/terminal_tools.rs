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

/// Settle heuristic: a command counts as finished once no output arrived for
/// this long. A heuristic, not prompt detection — interactive/TUI commands
/// may settle early; `timed_out` marks the capped case.
const EXEC_QUIET_MS: u64 = 700;
/// Default / max total wait for the blocking exec.
const EXEC_DEFAULT_WAIT_MS: u64 = 10_000;
const EXEC_MAX_WAIT_MS: u64 = 120_000;
/// Output caps: exec returns at most this much of the tail; read defaults to
/// 100 lines and caps at 500.
const EXEC_OUTPUT_CAP_BYTES: usize = 16 * 1024;
const READ_DEFAULT_LINES: usize = 100;
const READ_MAX_LINES: usize = 500;

impl SignalingBridge {
    /// HANDS-only (enforced at the dispatch layer). Runs `command` in the
    /// session's terminal and — unless `block` is false — returns the output
    /// captured until settle.
    pub async fn terminal_exec(
        &self,
        session_id: String,
        command: String,
        wait_ms: Option<u64>,
        block: Option<bool>,
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
        let cwd = match self.storage.lock().await.clone() {
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

        let offset = term.current_offset();
        term.write_input(format!("{command}\n").as_bytes())?;

        if block == Some(false) {
            return Ok(format!(
                "command started (not waiting). Use terminal_read to inspect output later.\n$ {command}"
            ));
        }

        let max_ms = wait_ms
            .unwrap_or(EXEC_DEFAULT_WAIT_MS)
            .clamp(EXEC_QUIET_MS, EXEC_MAX_WAIT_MS);
        let (bytes, timed_out) = term.wait_settle(offset, EXEC_QUIET_MS, max_ms).await;
        let mut output = String::from_utf8_lossy(&bytes).into_owned();
        if output.len() > EXEC_OUTPUT_CAP_BYTES {
            let cut = output.len() - EXEC_OUTPUT_CAP_BYTES;
            // Keep the tail — that's where the result and the prompt are.
            output = format!(
                "[…{cut} bytes trimmed…]\n{}",
                &output[output.len() - EXEC_OUTPUT_CAP_BYTES..]
            );
        }
        if timed_out {
            output.push_str(&format!(
                "\n[still producing output after {max_ms}ms — command may be long-running; \
                 use terminal_read for later output or terminal_exec with a larger wait_ms]"
            ));
        }
        Ok(output)
    }

    /// Both agents. Tail of the terminal scrollback as lossy UTF-8 —
    /// evidence-grade text agents can paste into chat or IPAV docs. Reads a
    /// dead (exited) terminal's retained scrollback too.
    pub async fn terminal_read(&self, session_id: String, lines: Option<u64>) -> Result<String> {
        let registry = self
            .terminal_registry()
            .ok_or_else(|| anyhow!("terminal registry not initialized (app still starting?)"))?;
        let Some(term) = registry.get_any(&session_id).await else {
            return Ok("no terminal has been started for this session".to_string());
        };
        let (snapshot, _, _) = term.open_view();
        let text = String::from_utf8_lossy(&snapshot);
        let n = lines.unwrap_or(READ_DEFAULT_LINES as u64) as usize;
        let n = n.clamp(1, READ_MAX_LINES);
        let all: Vec<&str> = text.lines().collect();
        let tail = &all[all.len().saturating_sub(n)..];
        Ok(tail.join("\n"))
    }
}

#[cfg(test)]
mod tests {
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
            .terminal_exec("s1".into(), "git push origin main".into(), None, None)
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

    #[tokio::test]
    async fn terminal_exec_rejects_empty_and_multiline() {
        let bridge = SignalingBridge::new();
        bridge.set_terminal_registry(Arc::new(TerminalRegistry::new()));
        assert!(bridge
            .terminal_exec("s1".into(), "  ".into(), None, None)
            .await
            .is_err());
        assert!(bridge
            .terminal_exec("s1".into(), "echo a\necho b".into(), None, None)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn terminal_read_without_terminal_is_clean() {
        let bridge = SignalingBridge::new();
        bridge.set_terminal_registry(Arc::new(TerminalRegistry::new()));
        let out = bridge.terminal_read("s1".into(), None).await.unwrap();
        assert!(out.contains("no terminal"), "got: {out}");
    }

    /// Full blocking path through a REAL shell: exec captures settled output,
    /// then terminal_read sees the same scrollback. Uses the user's `$SHELL`
    /// (the production spawn path) — assertions stay loose on prompts/rc noise
    /// and only look for the echoed marker.
    #[tokio::test(flavor = "multi_thread")]
    async fn terminal_exec_blocking_then_read_round_trip() {
        let bridge = SignalingBridge::new();
        bridge.set_terminal_registry(Arc::new(TerminalRegistry::new()));
        let out = bridge
            .terminal_exec(
                "s1".into(),
                "echo bothq-exec-marker".into(),
                Some(15_000),
                None,
            )
            .await
            .expect("exec should succeed");
        assert!(
            out.contains("bothq-exec-marker"),
            "settled output missing marker: {out:?}"
        );
        let read = bridge.terminal_read("s1".into(), None).await.unwrap();
        assert!(
            read.contains("bothq-exec-marker"),
            "terminal_read missing marker: {read:?}"
        );
    }
}
