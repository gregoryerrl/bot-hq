// Package autoinstall wires the trio Stop-hook (outboundhook) + PreToolUse-Bash
// hook (toolgate) installers into the bot-hq MCP server startup flow.
// Phase M M-1 c2 — eliminates the manual `bot-hq install-trio-hook` +
// `bot-hq install-toolgate-hook` invocation gap that produced Phase L
// Finding-1 (installer-not-run on this machine, settings.json had only
// the Stop hook absent the PreToolUse hook).
//
// Design notes:
//   - Best-effort: install failures write stderr warnings but do NOT fatally
//     exit the MCP server. The MCP server must keep serving even when the
//     installer can't write settings.json (e.g., $HOME unwritable, settings
//     file permission denied). Manual `bot-hq install-*` subcommands remain
//     available for explicit re-install.
//   - Idempotent + non-clobbering: defers to the existing InstallTrioHook
//     helpers in outboundhook + toolgate which already implement the
//     idempotent + non-clobbering write semantics. Repeated MCP-server
//     restarts converge to a single hook entry per matcher.
//   - Settings + binary paths are caller-injected (not resolved internally)
//     so the helper is testable without touching real $HOME.
package autoinstall

import (
	"fmt"
	"io"

	"github.com/gregoryerrl/bot-hq/internal/outboundhook"
	"github.com/gregoryerrl/bot-hq/internal/sessionstarthook"
	"github.com/gregoryerrl/bot-hq/internal/toolgate"
	"github.com/gregoryerrl/bot-hq/internal/voicemirror"
)

// Run installs both the OUTBOUND-MISS Stop hook and the K-16/L-5
// PreToolUse-Bash toolgate hook into the given settings.json path,
// referencing botHQPath as the hook command. Errors from either
// installer are written to warn (best-effort); the function returns nil
// always so callers can wire it into MCP startup without conditionally
// blocking the server.
//
// The two installers are independent (different hook event classes
// targeting different settings.json keys); a failure on one does not
// invalidate the other. Caller-side wiring is a single Run() call at
// MCP startup.
//
// Per Phase M M-1 c2 audit-doc + phase-m.md§Tier-1 row M-1 (ii)
// auto-install integration sub-item.
func Run(settingsPath, botHQPath string, warn io.Writer) {
	if settingsPath == "" || botHQPath == "" {
		fmt.Fprintf(warn, "autoinstall: skipped — settings or binary path empty\n")
		return
	}

	if err := outboundhook.InstallTrioHook(settingsPath, botHQPath); err != nil {
		fmt.Fprintf(warn, "autoinstall: outbound-miss Stop hook install failed (best-effort): %v\n", err)
	}

	if err := toolgate.InstallTrioHook(settingsPath, botHQPath); err != nil {
		fmt.Fprintf(warn, "autoinstall: toolgate PreToolUse hook install failed (best-effort): %v\n", err)
	}

	// Phase P P-11 follow-up (per user msg 15080 "usable in 1 rebuild+restart"):
	// auto-install SessionStart hook so /api/session-open delivers
	// rules-resolved + bootstrap + tasks to every Claude session at boot.
	// Without this hook, the v3.x USER-EXERCISE point #2 (edit-rule →
	// fresh-session abides) cannot close-loop. Best-effort + idempotent
	// per existing installer pattern.
	if err := sessionstarthook.InstallTrioHook(settingsPath, botHQPath); err != nil {
		fmt.Fprintf(warn, "autoinstall: session-start hook install failed (best-effort): %v\n", err)
	}

	// Phase N v2 R40 voice-mirror discipline hook — also auto-install per
	// the same "user shouldn't need to run separate install commands"
	// principle. Hooks fire alert-only; no behavior break if missing but
	// user-artifact write-discipline has reduced enforcement.
	if err := voicemirror.InstallTrioHook(settingsPath, botHQPath); err != nil {
		fmt.Fprintf(warn, "autoinstall: voice-mirror hook install failed (best-effort): %v\n", err)
	}
}
