package main

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/gregoryerrl/bot-hq/internal/outboundhook"
	"github.com/gregoryerrl/bot-hq/internal/sessionstarthook"
	"github.com/gregoryerrl/bot-hq/internal/toolgate"
	"github.com/gregoryerrl/bot-hq/internal/voicemirror"
)

// runOutboundMissHook is the Claude Code Stop-hook entry. Reads the
// hook input JSON from stdin (transcript_path et al), evaluates the
// three-clause filter, emits an OUTBOUND-MISS alert via the hub when
// the agent produced pane text without a hub_send tool call, AND blocks
// the stop event via {decision:block} JSON stdout output + ExitBlock=2
// + stderr propagation when shouldFlag fires. Phase M M-2 c1 R36 OUTBOUND-
// DISCIPLINE-MECHANICAL enforcement-conversion (mirrors R33 toolgate
// gate-CHECK exit-code propagation pattern).
func runOutboundMissHook() {
	os.Exit(outboundhook.RunHook(os.Stdin, os.Stdout, os.Stderr))
}

// runInstallTrioHook installs the OUTBOUND-MISS Stop hook into the
// trio agent's Claude settings.json. Idempotent + non-clobbering.
//
// Usage:
//
//	bot-hq install-trio-hook            # writes ~/.claude/settings.json
//	bot-hq install-trio-hook <path>     # writes a custom path
//
// User must additionally export BOT_HQ_AGENT_ID=<id> in the agent's
// pane environment so the hook knows which agent it is firing for.
func runInstallTrioHook() {
	settingsPath := ""
	if len(os.Args) > 2 {
		settingsPath = os.Args[2]
	}
	if settingsPath == "" {
		home, err := os.UserHomeDir()
		if err != nil {
			fmt.Fprintf(os.Stderr, "resolve home dir: %v\n", err)
			os.Exit(1)
		}
		settingsPath = filepath.Join(home, ".claude", "settings.json")
	}
	botHQPath, err := os.Executable()
	if err != nil || botHQPath == "" {
		fmt.Fprintf(os.Stderr, "resolve bot-hq binary path: %v\n", err)
		os.Exit(1)
	}
	if err := outboundhook.InstallTrioHook(settingsPath, botHQPath); err != nil {
		fmt.Fprintf(os.Stderr, "install: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("OUTBOUND-MISS hook installed in %s\n", settingsPath)
	fmt.Printf("Hook command: %s\n", outboundhook.SettingsHookCommand(botHQPath))
	fmt.Printf("Reminder: autostart trio panes set BOT_HQ_AGENT_ID automatically. For panes launched outside autostart (manual claude exec), export BOT_HQ_AGENT_ID=<id> before launch.\n")
}

// runToolPermissionHook is the PreToolUse hook entry point for the K-16
// class-split gate. Reads PreToolUse hook input from stdin, applies the
// gate, exits with 0 (allow) or 2 (block).
func runToolPermissionHook() {
	os.Exit(toolgate.RunHook(os.Stdin, os.Stderr))
}

// runVoiceMirrorHook is the Phase N v2 #3 N-2 PreToolUse hook entry
// point per R40 VOICE-MIRROR-DISCIPLINE. Reads JSON from stdin (Claude
// Code PreToolUse Write event payload), invokes voicemirror.RunHook
// which is alert-only (NOT blocking) — always exits 0.
func runVoiceMirrorHook() {
	os.Exit(voicemirror.RunHook(os.Stdin, os.Stderr))
}

// runInstallVoiceMirrorHook installs the Phase N v2 #3 N-2 PreToolUse-
// Write hook into the trio agent's Claude settings.json per R40 VOICE-
// MIRROR-DISCIPLINE. Idempotent + non-clobbering, mirroring
// runInstallToolgateHook + runInstallTrioHook patterns.
//
// Usage:
//
//	bot-hq install-voice-mirror-hook            # writes ~/.claude/settings.json
//	bot-hq install-voice-mirror-hook <path>     # writes a custom path
//
// Phase N v2 #8 close-composite — folds install subcommand per Rain
// Q2 lean (b1) at #3 N-2 PASS-2.
func runInstallVoiceMirrorHook() {
	settingsPath := ""
	if len(os.Args) > 2 {
		settingsPath = os.Args[2]
	}
	if settingsPath == "" {
		home, err := os.UserHomeDir()
		if err != nil {
			fmt.Fprintf(os.Stderr, "resolve home dir: %v\n", err)
			os.Exit(1)
		}
		settingsPath = filepath.Join(home, ".claude", "settings.json")
	}
	botHQPath, err := os.Executable()
	if err != nil || botHQPath == "" {
		fmt.Fprintf(os.Stderr, "resolve bot-hq binary path: %v\n", err)
		os.Exit(1)
	}
	if err := voicemirror.InstallTrioHook(settingsPath, botHQPath); err != nil {
		fmt.Fprintf(os.Stderr, "install: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("Voice-mirror PreToolUse-Write hook installed in %s\n", settingsPath)
	fmt.Printf("Hook command: %s\n", voicemirror.SettingsHookCommand(botHQPath))
	fmt.Printf("Hook fires on Write tool calls against user-artifact paths per R40 VOICE-MIRROR-DISCIPLINE (alert-only, NOT blocking).\n")
	fmt.Printf("INCLUDE patterns: ~/Documents/*, ~/Desktop/*, ~/.bot-hq/projects/<project>/{plans,eod,clips}/*, CLAUDE.md, README.md\n")
	fmt.Printf("SKIP patterns: **/memory/**, .git/, .cache/, node_modules/\n")
	fmt.Printf("Log: ~/.bot-hq/voice-mirror-log.md (override via BOT_HQ_VOICE_MIRROR_LOG_PATH env).\n")
	fmt.Printf("Hook activation requires Claude session-restart (settings.json not hot-reloaded mid-session per Phase L Finding-3).\n")
}

// runInstallSessionStartHook installs the Phase N v3.x-1.5 SessionStart
// hook into the trio agent's Claude settings.json. The installed hook
// command invokes `bot-hq session-open` at session-start, which fetches
// the daemon's /api/session-open and prints markdown the harness
// prepends as system-prompt context (overview + bootstrap + resolved
// rules + tasks). Idempotent + non-clobbering, mirroring
// runInstallTrioHook + runInstallToolgateHook + runInstallVoiceMirrorHook.
//
// Usage:
//
//	bot-hq install-session-start-hook            # writes ~/.claude/settings.json
//	bot-hq install-session-start-hook <path>     # writes a custom path
//
// Phase N v3.x-1.5 design-spike (157ea7f) §2.2 specifies the hook
// invocation surface. v3.x-2 implementation landed the session-open
// subcommand (cmd/bot-hq/context_switch.go runSessionOpen); this
// subcommand wires it into Claude settings.json so the hook fires
// automatically at SessionStart instead of requiring manual
// settings.json editing.
func runInstallSessionStartHook() {
	settingsPath := ""
	if len(os.Args) > 2 {
		settingsPath = os.Args[2]
	}
	if settingsPath == "" {
		home, err := os.UserHomeDir()
		if err != nil {
			fmt.Fprintf(os.Stderr, "resolve home dir: %v\n", err)
			os.Exit(1)
		}
		settingsPath = filepath.Join(home, ".claude", "settings.json")
	}
	botHQPath, err := os.Executable()
	if err != nil || botHQPath == "" {
		fmt.Fprintf(os.Stderr, "resolve bot-hq binary path: %v\n", err)
		os.Exit(1)
	}
	if err := sessionstarthook.InstallTrioHook(settingsPath, botHQPath); err != nil {
		fmt.Fprintf(os.Stderr, "install: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("SessionStart hook installed in %s\n", settingsPath)
	fmt.Printf("Hook command: %s\n", sessionstarthook.SettingsHookCommand(botHQPath))
	fmt.Printf("Hook fires at Claude SessionStart and prepends bot-hq session-open output (overview + bootstrap + resolved rules + tasks) as system-prompt context.\n")
	fmt.Printf("Project context: $BOT_HQ_PROJECT env var (authoritative); falls back to cwd-inference / 'bot-hq' default.\n")
	fmt.Printf("Agent context: $BOT_HQ_AGENT env var; falls back to 'brian'.\n")
	fmt.Printf("Hook activation requires Claude session-restart (settings.json not hot-reloaded mid-session per Phase L Finding-3).\n")
}

// runInstallToolgateHook installs the K-16 PreToolUse class-split gate
// hook into the trio agent's Claude settings.json. Idempotent +
// non-clobbering, mirroring runInstallTrioHook's pattern.
//
// Usage:
//
//	bot-hq install-toolgate-hook            # writes ~/.claude/settings.json
//	bot-hq install-toolgate-hook <path>     # writes a custom path
func runInstallToolgateHook() {
	settingsPath := ""
	if len(os.Args) > 2 {
		settingsPath = os.Args[2]
	}
	if settingsPath == "" {
		home, err := os.UserHomeDir()
		if err != nil {
			fmt.Fprintf(os.Stderr, "resolve home dir: %v\n", err)
			os.Exit(1)
		}
		settingsPath = filepath.Join(home, ".claude", "settings.json")
	}
	botHQPath, err := os.Executable()
	if err != nil || botHQPath == "" {
		fmt.Fprintf(os.Stderr, "resolve bot-hq binary path: %v\n", err)
		os.Exit(1)
	}
	if err := toolgate.InstallTrioHook(settingsPath, botHQPath); err != nil {
		fmt.Fprintf(os.Stderr, "install: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("Toolgate PreToolUse-Bash hook installed in %s\n", settingsPath)
	fmt.Printf("Hook command: %s\n", toolgate.SettingsHookCommand(botHQPath))
	fmt.Printf("Gates active per BOT_HQ_AGENT_ID:\n")
	fmt.Printf("  rain → K-16 class-split (HANDS-execute blocked) + K-13 R12 commit-gate\n")
	fmt.Printf("  brian (or non-rain trio member) → L-5 R33 pre-commit + pre-push + pre-merge gate-CHECK\n")
	fmt.Printf("Hook activation requires Claude session-restart (settings.json not hot-reloaded mid-session).\n")
}
