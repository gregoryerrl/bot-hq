package toolgate

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

// TestInstallTrioHook_FreshSettings locks the install-into-empty-or-missing
// path: settings.json absent → created with PreToolUse-Bash hook.
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
	if matcher, _ := entry["matcher"].(string); matcher != "Bash" {
		t.Errorf("matcher: got %q, want %q", matcher, "Bash")
	}
}

// TestInstallTrioHook_Idempotent locks idempotency: re-install with same
// path = no duplicate entry.
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
	if len(preArr) != 1 {
		t.Errorf("idempotency broken: got %d PreToolUse entries, want 1", len(preArr))
	}
}

// TestInstallTrioHook_PreservesOtherHooks locks non-clobbering of unrelated
// event-type hooks (e.g., Stop hook from outboundhook).
func TestInstallTrioHook_PreservesOtherHooks(t *testing.T) {
	tmp := t.TempDir()
	settingsPath := filepath.Join(tmp, "settings.json")

	// Pre-populate with an existing Stop hook (e.g., outboundhook)
	pre := map[string]any{
		"hooks": map[string]any{
			"Stop": []any{
				map[string]any{
					"matcher": "",
					"hooks": []any{
						map[string]any{
							"type":    "command",
							"command": "/path/to/outbound-miss-hook",
						},
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

	// Stop hook should still be present.
	stopArr, ok := hooks["Stop"].([]any)
	if !ok || len(stopArr) == 0 {
		t.Fatalf("Stop hook clobbered: %v", hooks["Stop"])
	}
	stopEntry := stopArr[0].(map[string]any)
	stopInner := stopEntry["hooks"].([]any)
	stopCmd := stopInner[0].(map[string]any)["command"].(string)
	if stopCmd != "/path/to/outbound-miss-hook" {
		t.Errorf("Stop hook command altered: %q", stopCmd)
	}

	// PreToolUse hook should be added.
	preArr, ok := hooks["PreToolUse"].([]any)
	if !ok || len(preArr) == 0 {
		t.Errorf("PreToolUse hook not added: %v", hooks["PreToolUse"])
	}
}

// TestInstallTrioHook_PreservesOtherPreToolUseEntries locks non-clobbering
// of other PreToolUse entries (different matchers / different commands).
func TestInstallTrioHook_PreservesOtherPreToolUseEntries(t *testing.T) {
	tmp := t.TempDir()
	settingsPath := filepath.Join(tmp, "settings.json")

	// Pre-populate with an existing PreToolUse entry (different matcher + command)
	pre := map[string]any{
		"hooks": map[string]any{
			"PreToolUse": []any{
				map[string]any{
					"matcher": "Edit",
					"hooks": []any{
						map[string]any{
							"type":    "command",
							"command": "/path/to/some-other-hook",
						},
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
		t.Errorf("expected 2 PreToolUse entries (existing + new); got %d", len(preArr))
	}

	// Existing entry preserved.
	foundExisting := false
	foundNew := false
	for _, e := range preArr {
		em := e.(map[string]any)
		matcher, _ := em["matcher"].(string)
		inner := em["hooks"].([]any)
		hm := inner[0].(map[string]any)
		cmd, _ := hm["command"].(string)
		if matcher == "Edit" && cmd == "/path/to/some-other-hook" {
			foundExisting = true
		}
		if matcher == "Bash" && strings.HasSuffix(cmd, hookCommandSuffix) {
			foundNew = true
		}
	}
	if !foundExisting {
		t.Errorf("existing Edit hook not preserved")
	}
	if !foundNew {
		t.Errorf("new Bash K-16 hook not added")
	}
}

// TestInstallTrioHook_InvalidJSON locks the refuse-to-overwrite-invalid
// behavior: existing settings.json with parse error → error returned,
// file untouched.
func TestInstallTrioHook_InvalidJSON(t *testing.T) {
	tmp := t.TempDir()
	settingsPath := filepath.Join(tmp, "settings.json")

	bad := []byte("{ not valid json")
	if err := os.WriteFile(settingsPath, bad, 0o644); err != nil {
		t.Fatalf("write bad json: %v", err)
	}

	err := InstallTrioHook(settingsPath, "/usr/local/bin/bot-hq")
	if err == nil {
		t.Fatalf("expected error on invalid JSON, got nil")
	}

	// File should be untouched.
	got, _ := os.ReadFile(settingsPath)
	if string(got) != string(bad) {
		t.Errorf("settings.json modified on invalid JSON: got %q, want %q", got, bad)
	}
}

// TestInstallTrioHook_MissingArgs locks defensive error returns for
// missing required arguments.
func TestInstallTrioHook_MissingArgs(t *testing.T) {
	if err := InstallTrioHook("", "/path/to/bin"); err == nil {
		t.Errorf("expected error on empty settingsPath, got nil")
	}
	if err := InstallTrioHook("/path/to/settings.json", ""); err == nil {
		t.Errorf("expected error on empty botHQPath, got nil")
	}
}
