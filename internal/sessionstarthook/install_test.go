package sessionstarthook

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

// TestSettingsHookCommand locks the command-string format. Match must
// be exact for hookExists idempotency check.
func TestSettingsHookCommand(t *testing.T) {
	got := SettingsHookCommand("/usr/local/bin/bot-hq")
	want := "/usr/local/bin/bot-hq session-open"
	if got != want {
		t.Errorf("SettingsHookCommand: got %q, want %q", got, want)
	}
}

// TestInstallTrioHook_FreshSettings: settings.json absent → created
// with SessionStart hook entry.
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
	startArr, ok := hooks["SessionStart"].([]any)
	if !ok || len(startArr) == 0 {
		t.Fatalf("SessionStart array missing/empty: %v", hooks["SessionStart"])
	}
	entry, ok := startArr[0].(map[string]any)
	if !ok {
		t.Fatalf("entry wrong shape: %T", startArr[0])
	}
	if matcher, _ := entry["matcher"].(string); matcher != "" {
		t.Errorf("matcher: got %q, want empty (SessionStart matches all)", matcher)
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

// TestInstallTrioHook_Idempotent: re-install with same path-and-command
// = no duplicate entry.
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
	startArr := hooks["SessionStart"].([]any)
	count := 0
	wantCmd := SettingsHookCommand(botHQ)
	for _, entry := range startArr {
		em, ok := entry.(map[string]any)
		if !ok {
			continue
		}
		inner, _ := em["hooks"].([]any)
		for _, h := range inner {
			hm, _ := h.(map[string]any)
			if c, _ := hm["command"].(string); c == wantCmd {
				count++
			}
		}
	}
	if count != 1 {
		t.Errorf("idempotency broken: got %d entries with command %q, want 1", count, wantCmd)
	}
}

// TestInstallTrioHook_PreservesOtherHooks: existing PreToolUse + Stop
// hooks (toolgate, voicemirror, outboundhook) are preserved verbatim.
// Critical for co-existence with the existing trio hook installers.
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
				map[string]any{
					"matcher": "Write",
					"hooks": []any{
						map[string]any{"type": "command", "command": "/usr/local/bin/bot-hq voice-mirror-hook"},
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

	preArr, _ := hooks["PreToolUse"].([]any)
	if len(preArr) != 2 {
		t.Errorf("PreToolUse clobbered: expected 2 entries (Bash + Write), got %d", len(preArr))
	}
	stopArr, _ := hooks["Stop"].([]any)
	if len(stopArr) != 1 {
		t.Errorf("Stop hook clobbered: %v", hooks["Stop"])
	}
	startArr, _ := hooks["SessionStart"].([]any)
	if len(startArr) != 1 {
		t.Errorf("SessionStart entry not added: %v", hooks["SessionStart"])
	}
}

// TestInstallTrioHook_PreservesUnrelatedSessionStart: existing
// SessionStart entries with DIFFERENT commands are preserved (additive
// install, not exclusive).
func TestInstallTrioHook_PreservesUnrelatedSessionStart(t *testing.T) {
	tmp := t.TempDir()
	settingsPath := filepath.Join(tmp, "settings.json")

	pre := map[string]any{
		"hooks": map[string]any{
			"SessionStart": []any{
				map[string]any{
					"matcher": "",
					"hooks": []any{
						map[string]any{"type": "command", "command": "/some/other/tool init"},
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
	startArr := hooks["SessionStart"].([]any)
	if len(startArr) != 2 {
		t.Errorf("expected 2 SessionStart entries (other-tool + bot-hq), got %d", len(startArr))
	}

	commands := map[string]bool{}
	for _, entry := range startArr {
		em, _ := entry.(map[string]any)
		inner, _ := em["hooks"].([]any)
		for _, h := range inner {
			hm, _ := h.(map[string]any)
			if c, ok := hm["command"].(string); ok {
				commands[c] = true
			}
		}
	}
	if !commands["/some/other/tool init"] {
		t.Errorf("unrelated SessionStart entry lost: %v", startArr)
	}
	if !commands[SettingsHookCommand("/usr/local/bin/bot-hq")] {
		t.Errorf("bot-hq SessionStart entry not added: %v", startArr)
	}
}

// TestInstallTrioHook_BadJSON: invalid settings.json → error rather
// than silent overwrite.
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
