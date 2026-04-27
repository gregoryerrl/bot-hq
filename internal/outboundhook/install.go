package outboundhook

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
)

// hookCommandTemplate is the Stop-hook command line written into
// settings.json. Substitutes the absolute path of the bot-hq binary at
// install time so users moving/rebuilding the binary get a stable
// pointer (no $PATH dependence).
const hookCommandSuffix = "outbound-miss-hook"

// SettingsHookCommand returns the command string written into
// settings.json hooks. Exposed for tests to assert exact match.
func SettingsHookCommand(botHQPath string) string {
	return botHQPath + " " + hookCommandSuffix
}

// InstallTrioHook installs the OUTBOUND-MISS Stop hook into the given
// settings.json path. Idempotent: a Stop-hook entry with the exact same
// command string is left alone. Non-clobbering: existing unrelated
// hooks (other Stop matchers, other event types) are preserved
// verbatim. Missing settings.json → created. Invalid JSON → error
// (caller surfaces; no silent rewrite-corrupt). Read-only filesystem
// or permission-denied → propagated error.
//
// botHQPath should be the absolute path of the bot-hq binary (typically
// from os.Executable()) so the installed hook references a stable
// location regardless of $PATH state at hook-fire time.
func InstallTrioHook(settingsPath, botHQPath string) error {
	if settingsPath == "" {
		return errors.New("install-trio-hook: settings path required")
	}
	if botHQPath == "" {
		return errors.New("install-trio-hook: bot-hq binary path required")
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
	stopArr := getOrCreateStopArray(hooks)

	if hookExists(stopArr, hookCmd) {
		// Idempotent no-op — already installed.
		return nil
	}

	stopArr = append(stopArr, map[string]any{
		"matcher": "",
		"hooks": []any{
			map[string]any{
				"type":    "command",
				"command": hookCmd,
			},
		},
	})
	hooks["Stop"] = stopArr
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
	// Existing key with wrong shape — preserve original under a backup
	// would be over-engineering for v1. Caller already guards against
	// malformed JSON. Treat unexpected shape as empty and overwrite.
	return make(map[string]any)
}

func getOrCreateStopArray(hooks map[string]any) []any {
	raw, ok := hooks["Stop"]
	if !ok {
		return nil
	}
	if arr, ok := raw.([]any); ok {
		return arr
	}
	return nil
}

// hookExists scans a Stop-array for any entry whose inner hooks contain
// a command-type hook with a matching command string. Match is exact —
// a different binary path counts as not-installed (correct: post-rebuild
// path migration should re-install rather than silently use the stale
// pointer).
func hookExists(stopArr []any, wantCmd string) bool {
	for _, entry := range stopArr {
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
