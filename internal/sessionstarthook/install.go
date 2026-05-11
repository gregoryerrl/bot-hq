// Package sessionstarthook installs the Phase N v3.x-1.5 SessionStart
// hook into Claude Code's settings.json. The hook command invokes
// `bot-hq session-open` at session-start, which fetches the daemon's
// /api/session-open and prints markdown the harness prepends as system-
// prompt context (overview + bootstrap + resolved rules + tasks).
//
// Mirrors internal/voicemirror + internal/outboundhook installer
// patterns: idempotent, non-clobbering. Note: settings.json rewrite is
// not crash-atomic (os.WriteFile, not temp-file+rename) — same shape as
// sibling installers; codebase-wide retrofit candidate.
package sessionstarthook

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
)

// hookCommandSuffix is the bot-hq subcommand the SessionStart hook
// invokes. `session-open` already exists (cmd/bot-hq/context_switch.go
// runSessionOpen) and is the documented SessionStart handler per
// design-spike 157ea7f §2.2.
const hookCommandSuffix = "session-open"

// SettingsHookCommand returns the command string written into
// settings.json. Exposed for tests to assert exact match.
func SettingsHookCommand(botHQPath string) string {
	return botHQPath + " " + hookCommandSuffix
}

// InstallDuoHook installs the SessionStart hook into the given
// settings.json path. Idempotent: an entry whose command exactly
// matches SettingsHookCommand(botHQPath) is left alone. Non-clobbering:
// other SessionStart entries (different commands), other matchers, and
// other event types are preserved verbatim. Missing settings.json →
// created. Invalid JSON → error (caller surfaces; no silent rewrite-
// corrupt).
//
// Naming mirrors outboundhook.InstallDuoHook + voicemirror.InstallDuoHook
// (each package's primary duo installer is named "InstallDuoHook" by
// convention). SessionStart hook activation requires Claude session-
// restart per Phase L Finding-3 (settings.json not hot-reloaded).
func InstallDuoHook(settingsPath, botHQPath string) error {
	if settingsPath == "" {
		return errors.New("install-session-start-hook: settings path required")
	}
	if botHQPath == "" {
		return errors.New("install-session-start-hook: bot-hq binary path required")
	}

	if err := os.MkdirAll(filepath.Dir(settingsPath), 0o755); err != nil {
		return fmt.Errorf("create settings dir: %w", err)
	}

	var settings map[string]any
	data, err := os.ReadFile(settingsPath)
	switch {
	case err == nil && len(data) > 0:
		if err := json.Unmarshal(data, &settings); err != nil {
			return fmt.Errorf("parse %s: %w (refusing to overwrite invalid JSON; resolve manually)", settingsPath, err)
		}
	case err == nil || os.IsNotExist(err):
		settings = make(map[string]any)
	default:
		return fmt.Errorf("read %s: %w", settingsPath, err)
	}
	if settings == nil {
		settings = make(map[string]any)
	}

	hookCmd := SettingsHookCommand(botHQPath)
	hooks := getOrCreateHooksMap(settings)
	startArr := getOrCreateSessionStartArray(hooks)

	if hookExists(startArr, hookCmd) {
		return nil
	}

	startArr = append(startArr, map[string]any{
		"matcher": "",
		"hooks": []any{
			map[string]any{
				"type":    "command",
				"command": hookCmd,
			},
		},
	})
	hooks["SessionStart"] = startArr
	settings["hooks"] = hooks

	out, err := json.MarshalIndent(settings, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal settings: %w", err)
	}
	if err := os.WriteFile(settingsPath, out, 0o600); err != nil {
		return fmt.Errorf("write %s: %w", settingsPath, err)
	}
	return nil
}

func getOrCreateHooksMap(settings map[string]any) map[string]any {
	raw, ok := settings["hooks"]
	if !ok {
		return make(map[string]any)
	}
	if m, ok := raw.(map[string]any); ok {
		return m
	}
	return make(map[string]any)
}

func getOrCreateSessionStartArray(hooks map[string]any) []any {
	raw, ok := hooks["SessionStart"]
	if !ok {
		return nil
	}
	if arr, ok := raw.([]any); ok {
		return arr
	}
	return nil
}

func hookExists(startArr []any, wantCmd string) bool {
	for _, entry := range startArr {
		em, ok := entry.(map[string]any)
		if !ok {
			continue
		}
		inner, ok := em["hooks"].([]any)
		if !ok {
			continue
		}
		for _, h := range inner {
			hm, ok := h.(map[string]any)
			if !ok {
				continue
			}
			if t, _ := hm["type"].(string); t != "command" {
				continue
			}
			if c, _ := hm["command"].(string); c == wantCmd {
				return true
			}
		}
	}
	return false
}
