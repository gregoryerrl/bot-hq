package autoinstall

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// TestRun_FreshSettingsInstallsBothHooks verifies the M-1 c2 integration:
// when settings.json is absent, Run() creates it with BOTH the Stop-hook
// (outboundhook) and the PreToolUse-Bash hook (toolgate) wired.
func TestRun_FreshSettingsInstallsBothHooks(t *testing.T) {
	dir := t.TempDir()
	settingsPath := filepath.Join(dir, "settings.json")
	botHQPath := "/test/bot-hq"

	var warn bytes.Buffer
	Run(settingsPath, botHQPath, &warn)

	if warn.Len() != 0 {
		t.Errorf("Run on fresh settings should not warn; got %q", warn.String())
	}

	settings := readSettings(t, settingsPath)
	hooks, _ := settings["hooks"].(map[string]any)
	if hooks == nil {
		t.Fatalf("settings.hooks key absent post-Run; got %v", settings)
	}

	stopArr, _ := hooks["Stop"].([]any)
	if len(stopArr) == 0 {
		t.Errorf("Stop hook (outboundhook) not installed; got %v", hooks)
	}

	preArr, _ := hooks["PreToolUse"].([]any)
	if len(preArr) == 0 {
		t.Errorf("PreToolUse hook (toolgate) not installed; got %v", hooks)
	}

	// Phase P P-11 follow-up: SessionStart hook (sessionopen) + voicemirror PreToolUse hook now also installed.
	startArr, _ := hooks["SessionStart"].([]any)
	if len(startArr) == 0 {
		t.Errorf("SessionStart hook (sessionopen) not installed; got %v", hooks)
	}
	if len(preArr) < 2 {
		t.Errorf("PreToolUse should include both toolgate + voicemirror (≥2 entries); got %d", len(preArr))
	}
	_ = strings.Contains // appease linter (kept import for other tests)
}

// TestRun_Idempotent verifies calling Run twice with identical inputs
// does not duplicate hook entries (defers to underlying InstallTrioHook
// idempotency).
func TestRun_Idempotent(t *testing.T) {
	dir := t.TempDir()
	settingsPath := filepath.Join(dir, "settings.json")
	botHQPath := "/test/bot-hq"

	var warn bytes.Buffer
	Run(settingsPath, botHQPath, &warn)
	Run(settingsPath, botHQPath, &warn)

	if warn.Len() != 0 {
		t.Errorf("Run idempotent path should not warn; got %q", warn.String())
	}

	settings := readSettings(t, settingsPath)
	hooks, _ := settings["hooks"].(map[string]any)

	stopArr, _ := hooks["Stop"].([]any)
	preArr, _ := hooks["PreToolUse"].([]any)

	if len(stopArr) != 1 {
		t.Errorf("Stop array should have exactly 1 entry after 2 Run() calls; got %d entries", len(stopArr))
	}
	// PreToolUse expects 2 after Phase P P-11 follow-up (toolgate Bash + voicemirror Write).
	if len(preArr) != 2 {
		t.Errorf("PreToolUse array should have exactly 2 entries (toolgate + voicemirror) after 2 Run() calls; got %d entries", len(preArr))
	}
}

// TestRun_PreservesUnrelatedHooks verifies non-clobbering behavior: an
// existing user-configured PreToolUse hook (different command) is left
// alone; Run only adds the bot-hq trio hooks alongside.
func TestRun_PreservesUnrelatedHooks(t *testing.T) {
	dir := t.TempDir()
	settingsPath := filepath.Join(dir, "settings.json")
	botHQPath := "/test/bot-hq"

	// Pre-populate with an unrelated PreToolUse hook (e.g., user's own
	// approval-gate or analytics hook).
	pre := map[string]any{
		"hooks": map[string]any{
			"PreToolUse": []any{
				map[string]any{
					"matcher": "Edit",
					"hooks": []any{
						map[string]any{
							"type":    "command",
							"command": "/usr/local/bin/user-edit-hook.sh",
						},
					},
				},
			},
		},
	}
	data, _ := json.MarshalIndent(pre, "", "  ")
	if err := os.WriteFile(settingsPath, data, 0o644); err != nil {
		t.Fatal(err)
	}

	var warn bytes.Buffer
	Run(settingsPath, botHQPath, &warn)

	settings := readSettings(t, settingsPath)
	hooks, _ := settings["hooks"].(map[string]any)
	preArr, _ := hooks["PreToolUse"].([]any)

	// Should now have 3 entries: preserved Edit-matcher + new Bash-matcher (toolgate)
	// + new Write-matcher (voicemirror).
	if len(preArr) != 3 {
		t.Errorf("PreToolUse should have 3 entries (preserved + toolgate + voicemirror); got %d: %v", len(preArr), preArr)
	}

	// Stop hook should also be installed.
	stopArr, _ := hooks["Stop"].([]any)
	if len(stopArr) == 0 {
		t.Errorf("Stop hook (outboundhook) not installed alongside preserved unrelated hook")
	}
}

// TestRun_EmptyPathsSkipsWithWarning verifies the defensive guard: when
// either path is empty, Run skips with a stderr warning and does NOT
// touch the filesystem.
func TestRun_EmptyPathsSkipsWithWarning(t *testing.T) {
	cases := []struct {
		name         string
		settingsPath string
		botHQPath    string
	}{
		{"empty_settings_path", "", "/x/bot-hq"},
		{"empty_bothq_path", "/x/settings.json", ""},
		{"both_empty", "", ""},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			var warn bytes.Buffer
			Run(tc.settingsPath, tc.botHQPath, &warn)

			if !strings.Contains(warn.String(), "skipped") {
				t.Errorf("expected skip-warning for empty-path case; got %q", warn.String())
			}

			if tc.settingsPath != "" {
				if _, err := os.Stat(tc.settingsPath); !os.IsNotExist(err) {
					t.Errorf("settings.json should not have been touched; got err=%v", err)
				}
			}
		})
	}
}

// readSettings reads + parses settings.json from path; fails the test
// on read or parse error.
func readSettings(t *testing.T, path string) map[string]any {
	t.Helper()
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read settings.json: %v", err)
	}
	var settings map[string]any
	if err := json.Unmarshal(data, &settings); err != nil {
		t.Fatalf("parse settings.json: %v (content=%q)", err, string(data))
	}
	return settings
}
