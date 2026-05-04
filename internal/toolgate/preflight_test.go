package toolgate

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// writeSettingsFile is a test helper that writes raw JSON content to a
// temp settings.json path and returns the path.
func writeSettingsFile(t *testing.T, content string) string {
	t.Helper()
	dir := t.TempDir()
	path := filepath.Join(dir, "settings.json")
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatalf("write fixture: %v", err)
	}
	return path
}

// TestVerifyHookInstallation_Fixtures locks the 6-fixture coverage axis
// per Phase M M-1 (i) design-spike v1.1 §3 L1 + Q6 (~10-12 rows).
//
// Fixture rows:
//  1. settings.json absent → CRITICAL
//  2. malformed JSON → CRITICAL
//  3. hooks key absent → CRITICAL
//  4. PreToolUse array absent → CRITICAL
//  5. hook present but partial-substring (e.g., "bot-hq" without
//     "tool-permission-hook") → WARNING (locks ALL-AND semantics)
//  6. hook present and correct (both substrings) → PASS
func TestVerifyHookInstallation_Fixtures(t *testing.T) {
	expected := []string{"bot-hq", "tool-permission-hook"}

	cases := []struct {
		name           string
		setup          func(t *testing.T) string
		wantStatus     Status
		wantFindingSub string
	}{
		{
			name: "settings_json_absent",
			setup: func(t *testing.T) string {
				dir := t.TempDir()
				return filepath.Join(dir, "missing-settings.json")
			},
			wantStatus:     StatusCritical,
			wantFindingSub: "missing",
		},
		{
			name: "malformed_json",
			setup: func(t *testing.T) string {
				return writeSettingsFile(t, `{"this is not": valid json`)
			},
			wantStatus:     StatusCritical,
			wantFindingSub: "parse error",
		},
		{
			name: "hooks_key_absent",
			setup: func(t *testing.T) string {
				return writeSettingsFile(t, `{"permissions":{"defaultMode":"default"}}`)
			},
			wantStatus:     StatusCritical,
			wantFindingSub: "hooks key absent",
		},
		{
			name: "pretooluse_array_absent",
			setup: func(t *testing.T) string {
				return writeSettingsFile(t, `{"hooks":{"Stop":[{"matcher":"","hooks":[{"type":"command","command":"/x"}]}]}}`)
			},
			wantStatus:     StatusCritical,
			wantFindingSub: "PreToolUse",
		},
		{
			name: "hook_present_partial_substring_locks_AND_semantics",
			setup: func(t *testing.T) string {
				// Command contains "bot-hq" but NOT "tool-permission-hook" — partial match.
				// Per ALL-AND semantics this must be WARNING, not PASS.
				return writeSettingsFile(t, `{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"/Users/foo/Projects/bot-hq/bot-hq install-toolgate-hook"}]}]}}`)
			},
			wantStatus:     StatusWarning,
			wantFindingSub: "no command contains all expected substrings",
		},
		{
			name: "hook_present_correct_both_substrings",
			setup: func(t *testing.T) string {
				return writeSettingsFile(t, `{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"/Users/foo/Projects/bot-hq/bot-hq tool-permission-hook"}]}]}}`)
			},
			wantStatus: StatusPass,
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			path := tc.setup(t)
			got := VerifyHookInstallation(path, expected)
			if got.Status != tc.wantStatus {
				t.Errorf("status = %v, want %v; findings=%v", got.Status, tc.wantStatus, got.Findings)
			}
			if tc.wantFindingSub != "" {
				if !findingsContain(got.Findings, tc.wantFindingSub) {
					t.Errorf("findings %v missing substring %q", got.Findings, tc.wantFindingSub)
				}
			}
		})
	}
}

// TestVerifyAgentEnv_Fixtures locks the 5-fixture env axis per design-spike
// §3 L1 + Brian PASS-1 BRAIN-add (multi-agent-id whitelist) + Rain v1.1
// emma-scope (emma WARNING not CRITICAL).
//
// Fixture rows:
//  1. env absent → CRITICAL
//  2. env "emma" → WARNING (emma is gemma-based; whitelist-fail intentional)
//  3. env "typo-value" → WARNING (general whitelist-fail)
//  4. env "brian" → PASS
//  5. env "rain"  → PASS
func TestVerifyAgentEnv_Fixtures(t *testing.T) {
	cases := []struct {
		name           string
		envValue       string
		envSet         bool
		wantStatus     Status
		wantFindingSub string
	}{
		{
			name:           "env_absent",
			envSet:         false,
			wantStatus:     StatusCritical,
			wantFindingSub: "absent",
		},
		{
			name:           "env_emma_warning_class",
			envSet:         true,
			envValue:       "emma",
			wantStatus:     StatusWarning,
			wantFindingSub: "emma is gemma-based",
		},
		{
			name:           "env_typo_value_warning",
			envSet:         true,
			envValue:       "discord",
			wantStatus:     StatusWarning,
			wantFindingSub: "whitelist",
		},
		{
			name:       "env_brian_pass",
			envSet:     true,
			envValue:   "brian",
			wantStatus: StatusPass,
		},
		{
			name:       "env_rain_pass",
			envSet:     true,
			envValue:   "rain",
			wantStatus: StatusPass,
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if tc.envSet {
				t.Setenv("BOT_HQ_AGENT_ID", tc.envValue)
			} else {
				// t.Setenv("","") would set empty rather than unset; use Unsetenv.
				if err := os.Unsetenv("BOT_HQ_AGENT_ID"); err != nil {
					t.Fatalf("unset env: %v", err)
				}
			}
			got := VerifyAgentEnv()
			if got.Status != tc.wantStatus {
				t.Errorf("status = %v, want %v; findings=%v", got.Status, tc.wantStatus, got.Findings)
			}
			if got.AgentID != tc.envValue {
				t.Errorf("AgentID = %q, want %q", got.AgentID, tc.envValue)
			}
			if tc.wantFindingSub != "" {
				if !findingsContain(got.Findings, tc.wantFindingSub) {
					t.Errorf("findings %v missing substring %q", got.Findings, tc.wantFindingSub)
				}
			}
		})
	}
}

// TestRunPreflight_CombinedWorstCase locks combined-Verdict semantics:
// worst-case Status across both checks; merged Findings; AgentID from env.
//
// Per design-spike §3 L1 RunPreflight + Q6 integration row.
func TestRunPreflight_CombinedWorstCase(t *testing.T) {
	cases := []struct {
		name        string
		settings    string
		envSet      bool
		envValue    string
		wantStatus  Status
		wantAgentID string
		wantNumFind int
	}{
		{
			name:        "both_pass",
			settings:    `{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"/x/bot-hq tool-permission-hook"}]}]}}`,
			envSet:      true,
			envValue:    "brian",
			wantStatus:  StatusPass,
			wantAgentID: "brian",
			wantNumFind: 0,
		},
		{
			name:        "hook_warning_env_critical_yields_critical",
			settings:    `{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"/x/different-hook"}]}]}}`,
			envSet:      false,
			wantStatus:  StatusCritical,
			wantAgentID: "",
			wantNumFind: 2,
		},
		{
			name:        "hook_pass_env_warning_yields_warning",
			settings:    `{"hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"/x/bot-hq tool-permission-hook"}]}]}}`,
			envSet:      true,
			envValue:    "emma",
			wantStatus:  StatusWarning,
			wantAgentID: "emma",
			wantNumFind: 1,
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			path := writeSettingsFile(t, tc.settings)
			if tc.envSet {
				t.Setenv("BOT_HQ_AGENT_ID", tc.envValue)
			} else {
				if err := os.Unsetenv("BOT_HQ_AGENT_ID"); err != nil {
					t.Fatalf("unset env: %v", err)
				}
			}
			got := RunPreflight(path)
			if got.Status != tc.wantStatus {
				t.Errorf("status = %v, want %v; findings=%v", got.Status, tc.wantStatus, got.Findings)
			}
			if got.AgentID != tc.wantAgentID {
				t.Errorf("AgentID = %q, want %q", got.AgentID, tc.wantAgentID)
			}
			if len(got.Findings) != tc.wantNumFind {
				t.Errorf("len(Findings) = %d, want %d; findings=%v", len(got.Findings), tc.wantNumFind, got.Findings)
			}
		})
	}
}

// TestStatusString locks the canonical PASS/WARNING/CRITICAL labels
// used in CLI output + hub-message formatting per Layer-2 surface.
func TestStatusString(t *testing.T) {
	cases := []struct {
		s    Status
		want string
	}{
		{StatusPass, "PASS"},
		{StatusWarning, "WARNING"},
		{StatusCritical, "CRITICAL"},
	}
	for _, tc := range cases {
		if got := tc.s.String(); got != tc.want {
			t.Errorf("Status(%d).String() = %q, want %q", tc.s, got, tc.want)
		}
	}
}

// TestDefaultSettingsPath verifies the canonical ~/.claude/settings.json
// resolution against $HOME.
func TestDefaultSettingsPath(t *testing.T) {
	got, err := DefaultSettingsPath()
	if err != nil {
		t.Fatalf("DefaultSettingsPath: %v", err)
	}
	if !strings.HasSuffix(got, filepath.Join(".claude", "settings.json")) {
		t.Errorf("DefaultSettingsPath = %q, want suffix %q", got, filepath.Join(".claude", "settings.json"))
	}
}

func findingsContain(findings []string, sub string) bool {
	for _, f := range findings {
		if strings.Contains(f, sub) {
			return true
		}
	}
	return false
}
