package toolgate

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// Status is the verdict severity returned by preflight checks.
//
// Phase M M-1 (i) preflight self-check.
type Status int

const (
	StatusPass Status = iota
	StatusWarning
	StatusCritical
)

// String returns the canonical PASS/WARNING/CRITICAL label.
func (s Status) String() string {
	switch s {
	case StatusPass:
		return "PASS"
	case StatusWarning:
		return "WARNING"
	case StatusCritical:
		return "CRITICAL"
	default:
		return "UNKNOWN"
	}
}

// Verdict is the result of a preflight check.
//
// Findings is empty on PASS; populated with human-readable failure
// descriptions on WARNING/CRITICAL. AgentID is the observed
// BOT_HQ_AGENT_ID value from VerifyAgentEnv (empty if env absent).
type Verdict struct {
	Status   Status
	Findings []string
	AgentID  string
}

// trioAgentWhitelist enumerates BOT_HQ_AGENT_ID values that consume
// ~/.claude/settings.json hooks (Claude-Code trio agents). Emma is
// gemma-based and does NOT consume settings.json hooks — emma agent-id
// fails whitelist as WARNING (not CRITICAL) since emma does not require
// toolgate enforcement. Per Phase M M-1 (i) design-spike v1.1 §1 + §3 L1.
var trioAgentWhitelist = []string{"brian", "rain"}

// VerifyHookInstallation reads settingsPath, parses JSON, navigates
// PreToolUse[].hooks[], returns Verdict.
//
// Match strategy: ALL substrings in expectedSubstrings MUST be present
// (logical AND) in some single PreToolUse hook command-string.
// Example: expectedSubstrings = []string{"bot-hq", "tool-permission-hook"}
// → both must appear in the same command for PASS. Partial-match (one
// substring present, other absent) does NOT pass.
//
// Verdict mapping:
//   - PASS:     some PreToolUse hook command contains ALL expected substrings
//   - WARNING:  PreToolUse entries present but no command has all substrings
//               (hook-config-exists-but-bot-hq-toolgate-not-installed-or-stale)
//   - CRITICAL: file missing / read error / parse error / hooks-key-absent /
//               PreToolUse-array-absent-or-empty
//
// Substring matching is path-tolerant by design — the bot-hq binary
// path varies per machine, so we check command-name substrings rather
// than exact paths. See design-spike §3 L1 substring-vs-exact-path lean.
func VerifyHookInstallation(settingsPath string, expectedSubstrings []string) Verdict {
	v := Verdict{Status: StatusPass}

	data, err := os.ReadFile(settingsPath)
	if err != nil {
		v.Status = StatusCritical
		if os.IsNotExist(err) {
			v.Findings = append(v.Findings, fmt.Sprintf("settings.json missing at %s", settingsPath))
		} else {
			v.Findings = append(v.Findings, fmt.Sprintf("settings.json read error: %v", err))
		}
		return v
	}

	var settings map[string]any
	if err := json.Unmarshal(data, &settings); err != nil {
		v.Status = StatusCritical
		v.Findings = append(v.Findings, fmt.Sprintf("settings.json parse error: %v", err))
		return v
	}

	hooks, _ := settings["hooks"].(map[string]any)
	if hooks == nil {
		v.Status = StatusCritical
		v.Findings = append(v.Findings, "settings.json hooks key absent")
		return v
	}

	preArr, _ := hooks["PreToolUse"].([]any)
	if len(preArr) == 0 {
		v.Status = StatusCritical
		v.Findings = append(v.Findings, "settings.json PreToolUse array absent or empty")
		return v
	}

	for _, entry := range preArr {
		em, ok := entry.(map[string]any)
		if !ok {
			continue
		}
		inner, _ := em["hooks"].([]any)
		for _, h := range inner {
			hm, ok := h.(map[string]any)
			if !ok {
				continue
			}
			cmd, _ := hm["command"].(string)
			if cmd == "" {
				continue
			}
			if containsAll(cmd, expectedSubstrings) {
				return v
			}
		}
	}

	v.Status = StatusWarning
	v.Findings = append(v.Findings, fmt.Sprintf("PreToolUse hooks present but no command contains all expected substrings %v (toolgate hook not installed or stale-path)", expectedSubstrings))
	return v
}

// containsAll returns true iff every substring in subs is present in s.
// Empty subs slice trivially returns true.
func containsAll(s string, subs []string) bool {
	for _, sub := range subs {
		if !strings.Contains(s, sub) {
			return false
		}
	}
	return true
}

// VerifyAgentEnv reads BOT_HQ_AGENT_ID from os.Getenv.
//
// Verdict mapping:
//   - PASS:     env present and value ∈ trioAgentWhitelist {brian, rain}
//   - WARNING:  env present but value not in whitelist (e.g., "emma" or
//               typo). Emma is intentionally WARNING-class — emma is
//               gemma-based and does not require toolgate enforcement,
//               so emma misconfig flags but does not halt trio.
//   - CRITICAL: env absent (agent context unknown; cannot route alerts)
//
// Per Phase M M-1 (i) design-spike v1.1 §3 L1 + Brian PASS-1 BRAIN-add
// (msg 7418) + Rain v1.1 emma-scope clarification.
func VerifyAgentEnv() Verdict {
	v := Verdict{Status: StatusPass}
	id := os.Getenv("BOT_HQ_AGENT_ID")
	v.AgentID = id

	if id == "" {
		v.Status = StatusCritical
		v.Findings = append(v.Findings, "BOT_HQ_AGENT_ID env-var absent")
		return v
	}

	for _, valid := range trioAgentWhitelist {
		if id == valid {
			return v
		}
	}

	v.Status = StatusWarning
	v.Findings = append(v.Findings, fmt.Sprintf("BOT_HQ_AGENT_ID=%q not in Claude-Code-trio whitelist %v (emma is gemma-based; emma agent-id is WARNING-class, not CRITICAL)", id, trioAgentWhitelist))
	return v
}

// RunPreflight executes VerifyHookInstallation and VerifyAgentEnv,
// returns combined Verdict with worst-case Status, merged Findings,
// and AgentID set from env.
//
// Default expected substrings: {"bot-hq", hookCommandSuffix} where
// hookCommandSuffix == "tool-permission-hook" per install.go.
//
// Caller invariant: invoke AFTER hub_register. The trio's session-start
// flow registers first, THEN runs preflight, THEN emits hub_send /
// hub_flag if !PASS. Per design-spike §3 L3 register→preflight→emit
// ordering edge-case.
func RunPreflight(settingsPath string) Verdict {
	expected := []string{"bot-hq", hookCommandSuffix}
	hookV := VerifyHookInstallation(settingsPath, expected)
	envV := VerifyAgentEnv()

	combined := Verdict{AgentID: envV.AgentID}
	if hookV.Status > combined.Status {
		combined.Status = hookV.Status
	}
	if envV.Status > combined.Status {
		combined.Status = envV.Status
	}
	combined.Findings = append(combined.Findings, hookV.Findings...)
	combined.Findings = append(combined.Findings, envV.Findings...)
	return combined
}

// DefaultSettingsPath returns ~/.claude/settings.json — the canonical
// user-level Claude Code settings.json path.
func DefaultSettingsPath() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("preflight: resolve home dir: %w", err)
	}
	return filepath.Join(home, ".claude", "settings.json"), nil
}
