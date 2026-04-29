package toolgate

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
)

// hookCommandSuffix is the bot-hq subcommand that invokes RunHook.
// The full hook command written into settings.json is
// `<bot-hq-binary-path> <hookCommandSuffix>`.
const hookCommandSuffix = "tool-permission-hook"

// SettingsHookCommand returns the command string written into
// settings.json hooks. Exposed for tests to assert exact match.
func SettingsHookCommand(botHQPath string) string {
	return botHQPath + " " + hookCommandSuffix
}

// InstallTrioHook installs the K-16 PreToolUse class-split gate hook
// into the given settings.json path. Idempotent: a PreToolUse-Bash hook
// with the exact same command string is left alone. Non-clobbering:
// existing unrelated hooks (other PreToolUse matchers, other event
// types like Stop) are preserved verbatim. Missing settings.json →
// created. Invalid JSON → error.
//
// botHQPath should be the absolute path of the bot-hq binary so the
// installed hook references a stable location regardless of $PATH state
// at hook-fire time.
//
// Phase K K-16 — mirrors outboundhook.InstallTrioHook pattern (Stop
// hook) for the PreToolUse / Bash class-split gate.
func InstallTrioHook(settingsPath, botHQPath string) error {
	if settingsPath == "" {
		return errors.New("install-toolgate-hook: settings path required")
	}
	if botHQPath == "" {
		return errors.New("install-toolgate-hook: bot-hq binary path required")
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
	preArr := getOrCreatePreToolUseArray(hooks)

	if hookExists(preArr, hookCmd) {
		// Idempotent no-op — already installed.
		return nil
	}

	preArr = append(preArr, map[string]any{
		"matcher": "Bash",
		"hooks": []any{
			map[string]any{
				"type":    "command",
				"command": hookCmd,
			},
		},
	})
	hooks["PreToolUse"] = preArr
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

func getOrCreatePreToolUseArray(hooks map[string]any) []any {
	raw, ok := hooks["PreToolUse"]
	if !ok {
		return nil
	}
	if arr, ok := raw.([]any); ok {
		return arr
	}
	return nil
}

// hookExists scans a PreToolUse-array for any entry whose inner hooks
// contain a command-type hook with a matching command string. Match is
// exact — different binary path counts as not-installed (correct;
// post-rebuild path migration should re-install rather than silently
// use the stale pointer).
func hookExists(preArr []any, wantCmd string) bool {
	for _, entry := range preArr {
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
