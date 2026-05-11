package voicemirror

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
)

// hookCommandSuffix is the bot-hq subcommand that invokes RunHook for
// the voice-mirror PreToolUse hook. Mirrors toolgate's
// hookCommandSuffix pattern + outboundhook InstallDuoHook precedent.
const hookCommandSuffix = "voice-mirror-hook"

// SettingsHookCommand returns the command string written into
// settings.json PreToolUse-Write hook entry. Exposed for tests to
// assert exact match.
func SettingsHookCommand(botHQPath string) string {
	return botHQPath + " " + hookCommandSuffix
}

// InstallDuoHook installs the Phase N v2 #3 N-2 voice-mirror
// PreToolUse-Write hook into the given settings.json path. Idempotent
// (existing hook with same command is left alone) + non-clobbering
// (other PreToolUse matchers + other event types preserved). Missing
// settings.json → created. Invalid JSON → error.
//
// botHQPath should be the absolute path of the bot-hq binary so the
// installed hook references a stable location regardless of $PATH at
// hook-fire time.
//
// Mirrors toolgate.InstallDuoHook pattern (which is a Bash hook) but
// installs a Write-matcher hook — voice-mirror discipline applies to
// Write tool calls against user-artifact paths per R40 +
// MatchesUserArtifactPath path-set.
//
// Phase N v2 #8 close-composite — folds install-voice-mirror-hook
// subcommand per Rain Q2 lean (b1) at PASS-2 of #3 N-2.
func InstallDuoHook(settingsPath, botHQPath string) error {
	if settingsPath == "" {
		return errors.New("install-voice-mirror-hook: settings path required")
	}
	if botHQPath == "" {
		return errors.New("install-voice-mirror-hook: bot-hq binary path required")
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
		return nil
	}

	preArr = append(preArr, map[string]any{
		"matcher": "Write",
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
