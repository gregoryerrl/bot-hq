package voicemirror

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func readSettings(t *testing.T, path string) map[string]any {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read settings: %v", err)
	}
	var settings map[string]any
	if err := json.Unmarshal(data, &settings); err != nil {
		t.Fatalf("parse settings: %v", err)
	}
	return settings
}

// TestSettingsHookCommand locks the command-string format used in
// settings.json hook entries. Match must be exact for hookExists
// idempotency check.
func TestSettingsHookCommand(t *testing.T) {
	got := SettingsHookCommand("/usr/local/bin/bot-hq")
	want := "/usr/local/bin/bot-hq voice-mirror-hook"
	if got != want {
		t.Errorf("SettingsHookCommand: got %q, want %q", got, want)
	}
}

// TestInstallTrioHook_FreshSettings locks install-into-missing-settings:
// settings.json absent → created with PreToolUse-Write hook.
func TestInstallTrioHook_FreshSettings(t *testing.T) {
	tmp := t.TempDir()
	settingsPath := filepath.Join(tmp, "settings.json")
	botHQ := "/usr/local/bin/bot-hq"

	if err := InstallTrioHook(settingsPath, botHQ); err != nil {
		t.Fatalf("InstallTrioHook: %v", err)
	}

	settings := readSettings(t, settingsPath)
	hooks, ok := settings["hooks"].(map[string]any)
	if !ok {
		t.Fatalf("hooks key missing/wrong shape: %T", settings["hooks"])
	}
	preArr, ok := hooks["PreToolUse"].([]any)
	if !ok || len(preArr) == 0 {
		t.Fatalf("PreToolUse array missing/empty: %v", hooks["PreToolUse"])
	}
	entry, ok := preArr[0].(map[string]any)
	if !ok {
		t.Fatalf("entry wrong shape: %T", preArr[0])
	}
	if matcher, _ := entry["matcher"].(string); matcher != "Write" {
		t.Errorf("matcher: got %q, want %q", matcher, "Write")
	}
	inner, ok := entry["hooks"].([]any)
	if !ok || len(inner) == 0 {
		t.Fatalf("inner hooks missing")
	}
	cmd, _ := inner[0].(map[string]any)["command"].(string)
	if cmd != SettingsHookCommand(botHQ) {
		t.Errorf("command: got %q, want %q", cmd, SettingsHookCommand(botHQ))
	}
}

// TestInstallTrioHook_Idempotent locks idempotency: re-install with
// same path-and-command = no duplicate entry.
func TestInstallTrioHook_Idempotent(t *testing.T) {
	tmp := t.TempDir()
	settingsPath := filepath.Join(tmp, "settings.json")
	botHQ := "/usr/local/bin/bot-hq"

	if err := InstallTrioHook(settingsPath, botHQ); err != nil {
		t.Fatalf("first InstallTrioHook: %v", err)
	}
	if err := InstallTrioHook(settingsPath, botHQ); err != nil {
		t.Fatalf("second InstallTrioHook: %v", err)
	}

	settings := readSettings(t, settingsPath)
	hooks := settings["hooks"].(map[string]any)
	preArr := hooks["PreToolUse"].([]any)
	count := 0
	for _, entry := range preArr {
		em, ok := entry.(map[string]any)
		if !ok {
			continue
		}
		if matcher, _ := em["matcher"].(string); matcher != "Write" {
			continue
		}
		count++
	}
	if count != 1 {
		t.Errorf("idempotency broken: got %d Write-matcher entries, want 1", count)
	}
}

// TestInstallTrioHook_PreservesOtherHooks locks non-clobbering: existing
// non-Write entries (other matchers, other event types like Stop) are
// preserved verbatim. Critical for co-existence with toolgate (Bash) +
// outboundhook (Stop).
func TestInstallTrioHook_PreservesOtherHooks(t *testing.T) {
	tmp := t.TempDir()
	settingsPath := filepath.Join(tmp, "settings.json")

	pre := map[string]any{
		"hooks": map[string]any{
			"PreToolUse": []any{
				map[string]any{
					"matcher": "Bash",
					"hooks": []any{
						map[string]any{"type": "command", "command": "/usr/local/bin/bot-hq tool-permission-hook"},
					},
				},
			},
			"Stop": []any{
				map[string]any{
					"matcher": "",
					"hooks": []any{
						map[string]any{"type": "command", "command": "/usr/local/bin/bot-hq outbound-miss-hook"},
					},
				},
			},
		},
	}
	data, _ := json.MarshalIndent(pre, "", "  ")
	os.WriteFile(settingsPath, data, 0o644)

	if err := InstallTrioHook(settingsPath, "/usr/local/bin/bot-hq"); err != nil {
		t.Fatalf("InstallTrioHook: %v", err)
	}

	settings := readSettings(t, settingsPath)
	hooks := settings["hooks"].(map[string]any)
	preArr := hooks["PreToolUse"].([]any)
	if len(preArr) != 2 {
		t.Errorf("expected 2 PreToolUse entries (Bash + Write); got %d", len(preArr))
	}
	stopArr, ok := hooks["Stop"].([]any)
	if !ok || len(stopArr) != 1 {
		t.Errorf("Stop hook clobbered: %v", hooks["Stop"])
	}

	// Verify both Bash + Write matchers present
	matchers := map[string]bool{}
	for _, e := range preArr {
		em := e.(map[string]any)
		matchers[em["matcher"].(string)] = true
	}
	if !matchers["Bash"] {
		t.Errorf("Bash matcher lost: %v", preArr)
	}
	if !matchers["Write"] {
		t.Errorf("Write matcher missing: %v", preArr)
	}
}

// TestInstallTrioHook_BadJSON locks defensive behavior on invalid
// settings.json: error rather than silent overwrite.
func TestInstallTrioHook_BadJSON(t *testing.T) {
	tmp := t.TempDir()
	settingsPath := filepath.Join(tmp, "settings.json")
	os.WriteFile(settingsPath, []byte("not json"), 0o644)

	err := InstallTrioHook(settingsPath, "/usr/local/bin/bot-hq")
	if err == nil {
		t.Fatalf("expected error on bad JSON; got nil")
	}
	if !strings.Contains(err.Error(), "parse") {
		t.Errorf("error should mention parse failure; got: %v", err)
	}
}

// TestInstallTrioHook_RequiresSettingsPath locks input validation.
func TestInstallTrioHook_RequiresSettingsPath(t *testing.T) {
	if err := InstallTrioHook("", "/usr/local/bin/bot-hq"); err == nil {
		t.Errorf("expected error on empty settingsPath; got nil")
	}
}

// TestInstallTrioHook_RequiresBotHQPath locks input validation.
func TestInstallTrioHook_RequiresBotHQPath(t *testing.T) {
	tmp := t.TempDir()
	settingsPath := filepath.Join(tmp, "settings.json")
	if err := InstallTrioHook(settingsPath, ""); err == nil {
		t.Errorf("expected error on empty botHQPath; got nil")
	}
}
