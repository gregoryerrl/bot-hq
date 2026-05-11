package toolgate

import (
	"bytes"
	"encoding/json"
	"strings"
	"testing"
)

func runHookWithInput(t *testing.T, input HookInput, agentID string) (int, string) {
	t.Helper()
	if agentID != "" {
		t.Setenv("BOT_HQ_AGENT_ID", agentID)
	} else {
		t.Setenv("BOT_HQ_AGENT_ID", "")
	}
	data, err := json.Marshal(input)
	if err != nil {
		t.Fatalf("marshal input: %v", err)
	}
	var stderr bytes.Buffer
	code := RunHook(bytes.NewReader(data), &stderr)
	return code, stderr.String()
}

// TestRunHook_RainExecutePatternBlocked locks the load-bearing K-16
// invariant: rain + Bash + HANDS-execute pattern → exit 2 + stderr
// message with pattern + recovery anchor.
func TestRunHook_RainExecutePatternBlocked(t *testing.T) {
	cases := []struct {
		command string
		pattern string
	}{
		{"git push origin main", "git push"},
		{`git commit -m "msg"`, "git commit"},
		{"gh pr create --title X", "gh pr"},
		{"gh issue create --title X", "gh issue"},
	}
	for _, tc := range cases {
		input := HookInput{
			ToolName:  "Bash",
			ToolInput: map[string]any{"command": tc.command},
		}
		code, stderr := runHookWithInput(t, input, "rain")
		if code != ExitBlock {
			t.Errorf("RunHook(rain, %q) = %d, want %d (block)", tc.command, code, ExitBlock)
		}
		if !strings.Contains(stderr, "K-16 class-split gate") {
			t.Errorf("stderr missing K-16 marker for %q: %q", tc.command, stderr)
		}
		if !strings.Contains(stderr, tc.pattern) {
			t.Errorf("stderr missing pattern %q for %q: %q", tc.pattern, tc.command, stderr)
		}
		if !strings.Contains(stderr, "discipline-anchors.md") {
			t.Errorf("stderr missing recovery anchor pointer for %q: %q", tc.command, stderr)
		}
	}
}

// TestRunHook_RainAllowedReadOnly locks that read-only commands rain
// legitimately uses (git status / gh pr view / etc.) are NOT blocked.
func TestRunHook_RainAllowedReadOnly(t *testing.T) {
	cases := []string{
		"git status",
		"git log --oneline -10",
		"git diff",
		"gh pr view 366",
		"gh issue list",
		"ls -la",
		"cat file.txt",
	}
	for _, cmd := range cases {
		input := HookInput{
			ToolName:  "Bash",
			ToolInput: map[string]any{"command": cmd},
		}
		code, stderr := runHookWithInput(t, input, "rain")
		if code != ExitAllow {
			t.Errorf("RunHook(rain, %q) = %d, want %d (allow); stderr: %q", cmd, code, ExitAllow, stderr)
		}
	}
}

// TestRunHook_BrianExecuteAllowed locks that brian (HANDS) is NOT
// blocked by the K-16 class-split gate even on execute patterns — those
// belong to brian's class. Isolates BOT_HQ_HOME to a tempdir so K-13 R12
// fail-soft (hubdb-absent) AND L-5 R33 fail-soft (gatefile-absent) both
// engage; this test is scoped to K-16 specifically. Per-gate behavior
// is covered by dedicated tests (r12_test.go for K-13; r33_test.go for L-5).
func TestRunHook_BrianExecuteAllowed(t *testing.T) {
	t.Setenv("BOT_HQ_HOME", t.TempDir())
	cases := []string{
		"git push origin main",
		`git commit -m "msg"`,
		"gh pr create --title X",
		"git push --force",
	}
	for _, cmd := range cases {
		input := HookInput{
			ToolName:  "Bash",
			ToolInput: map[string]any{"command": cmd},
		}
		code, stderr := runHookWithInput(t, input, "brian")
		if code != ExitAllow {
			t.Errorf("RunHook(brian, %q) = %d, want %d (allow); stderr: %q", cmd, code, ExitAllow, stderr)
		}
	}
}

// TestRunHook_NonBashTool locks that Edit / Write / Read / other tool
// calls are out of MVP scope (per Rain msg 6411) — always allowed
// regardless of agent ID.
func TestRunHook_NonBashTool(t *testing.T) {
	cases := []string{"Edit", "Write", "Read", "Glob", "Grep"}
	for _, tool := range cases {
		input := HookInput{
			ToolName:  tool,
			ToolInput: map[string]any{"file_path": "/tmp/foo"},
		}
		code, stderr := runHookWithInput(t, input, "rain")
		if code != ExitAllow {
			t.Errorf("RunHook(rain, tool=%s) = %d, want %d (allow; non-Bash out of MVP); stderr: %q",
				tool, code, ExitAllow, stderr)
		}
	}
}

// TestRunHook_NoAgentID locks the defensive default: missing
// BOT_HQ_AGENT_ID env var → allow with no warning. Non-duo Claude
// Code instances (e.g., ad-hoc local dev) shouldn't be blocked.
func TestRunHook_NoAgentID(t *testing.T) {
	input := HookInput{
		ToolName:  "Bash",
		ToolInput: map[string]any{"command": "git push origin main"},
	}
	code, _ := runHookWithInput(t, input, "")
	if code != ExitAllow {
		t.Errorf("RunHook(no-agent-id) = %d, want %d (defensive allow)", code, ExitAllow)
	}
}

// TestRunHook_MalformedJSONInput locks defensive behavior: malformed
// hook input → allow (don't block agent on hook-side bugs).
func TestRunHook_MalformedJSONInput(t *testing.T) {
	t.Setenv("BOT_HQ_AGENT_ID", "rain")
	var stderr bytes.Buffer
	code := RunHook(bytes.NewReader([]byte("not json")), &stderr)
	if code != ExitAllow {
		t.Errorf("RunHook(malformed-json) = %d, want %d (defensive allow)", code, ExitAllow)
	}
}
