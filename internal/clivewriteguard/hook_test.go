package clivewriteguard

// Tests for the Clive write-API-only enforcement hook (P-4b /
// phase-n.md:545 + PhaseNv3CliveExpansion). Hook is BLOCKING (exit 2)
// for the canonical FS-write tool set when BOT_HQ_AGENT_ID=clive;
// no-op for any other agent.

import (
	"bytes"
	"strings"
	"testing"
)

// withAgentID temporarily sets the BOT_HQ_AGENT_ID env var and
// returns a cleanup func. Usage: defer withAgentID(t, "clive")()
func withAgentID(t *testing.T, val string) func() {
	t.Helper()
	t.Setenv(agentIDEnvVar, val)
	return func() {} // Setenv handles cleanup via t.Cleanup
}

// jsonInput builds a minimal HookInput JSON for the given tool +
// optional file_path / command fields.
func jsonInput(tool, filePath, command string) string {
	parts := []string{`"tool_name":"` + tool + `"`}
	if filePath != "" || command != "" {
		var fields []string
		if filePath != "" {
			fields = append(fields, `"file_path":"`+filePath+`"`)
		}
		if command != "" {
			fields = append(fields, `"command":"`+command+`"`)
		}
		parts = append(parts, `"tool_input":{`+strings.Join(fields, ",")+`}`)
	}
	return "{" + strings.Join(parts, ",") + "}"
}

// TestRunHook_NonClive_AllowsAll verifies the no-op path: when running
// as Brian or Rain or unset, every tool passes through.
func TestRunHook_NonClive_AllowsAll(t *testing.T) {
	cases := []string{"brian", "rain", "", "BRIAN"}
	for _, agent := range cases {
		t.Run("agent="+agent, func(t *testing.T) {
			withAgentID(t, agent)
			for _, tool := range []string{"Edit", "Write", "Bash", "MultiEdit", "Read"} {
				stdin := strings.NewReader(jsonInput(tool, "/tmp/x", ""))
				var stderr bytes.Buffer
				code := RunHook(stdin, &stderr)
				if code != ExitAllow {
					t.Errorf("agent=%q tool=%q got exit=%d, want %d (no-op for non-Clive); stderr=%s",
						agent, tool, code, ExitAllow, stderr.String())
				}
			}
		})
	}
}

// TestRunHook_Clive_BlocksFSWriteTools is the load-bearing test:
// confirms each FS-write tool is blocked for Clive with the reason
// surfaced to stderr.
func TestRunHook_Clive_BlocksFSWriteTools(t *testing.T) {
	withAgentID(t, "clive")
	cases := []struct {
		tool         string
		wantInReason string
	}{
		{"Edit", "POST /api/files/{path}/clive"},
		{"Write", "POST /api/files/{path}/clive"},
		{"MultiEdit", "POST /api/files/{path}/clive"},
		{"NotebookEdit", "POST /api/files/{path}/clive"},
	}
	for _, tc := range cases {
		t.Run(tc.tool, func(t *testing.T) {
			stdin := strings.NewReader(jsonInput(tc.tool, "/tmp/foo.md", ""))
			var stderr bytes.Buffer
			code := RunHook(stdin, &stderr)
			if code != ExitBlock {
				t.Errorf("tool=%q got exit=%d, want %d (block)", tc.tool, code, ExitBlock)
			}
			if !strings.Contains(stderr.String(), tc.wantInReason) {
				t.Errorf("tool=%q stderr missing reason substring %q; got: %s",
					tc.tool, tc.wantInReason, stderr.String())
			}
			if !strings.Contains(stderr.String(), "/tmp/foo.md") {
				t.Errorf("tool=%q stderr missing attempted path; got: %s", tc.tool, stderr.String())
			}
		})
	}
}

// TestRunHook_Clive_BlocksBash verifies Bash is fully blocked for
// Clive (canonical-store-write-API-only authority + no shell).
func TestRunHook_Clive_BlocksBash(t *testing.T) {
	withAgentID(t, "clive")
	stdin := strings.NewReader(jsonInput("Bash", "", "rm -rf /tmp/foo"))
	var stderr bytes.Buffer
	code := RunHook(stdin, &stderr)
	if code != ExitBlock {
		t.Errorf("got exit=%d, want %d (block Bash for Clive)", code, ExitBlock)
	}
	if !strings.Contains(stderr.String(), "Clive cannot use Bash") {
		t.Errorf("stderr missing Bash-block reason; got: %s", stderr.String())
	}
	if !strings.Contains(stderr.String(), "rm -rf /tmp/foo") {
		t.Errorf("stderr missing attempted command; got: %s", stderr.String())
	}
}

// TestRunHook_Clive_AllowsReadOnlyTools confirms Read / Grep / Glob /
// other non-write tools are NOT blocked — Clive needs investigation
// authority.
func TestRunHook_Clive_AllowsReadOnlyTools(t *testing.T) {
	withAgentID(t, "clive")
	tools := []string{"Read", "Grep", "Glob", "WebFetch", "WebSearch", "mcp__bot-hq__hub_send"}
	for _, tool := range tools {
		t.Run(tool, func(t *testing.T) {
			stdin := strings.NewReader(jsonInput(tool, "/tmp/x", ""))
			var stderr bytes.Buffer
			code := RunHook(stdin, &stderr)
			if code != ExitAllow {
				t.Errorf("tool=%q got exit=%d, want %d (allow); stderr=%s",
					tool, code, ExitAllow, stderr.String())
			}
		})
	}
}

// TestRunHook_Clive_CaseInsensitive verifies tool-name comparison
// tolerates harness casing variations (e.g., "EDIT" / "edit" / "Edit").
func TestRunHook_Clive_CaseInsensitive(t *testing.T) {
	withAgentID(t, "clive")
	for _, variant := range []string{"edit", "EDIT", "Edit", "  Edit  "} {
		stdin := strings.NewReader(jsonInput(variant, "/tmp/x", ""))
		var stderr bytes.Buffer
		code := RunHook(stdin, &stderr)
		if code != ExitBlock {
			t.Errorf("variant=%q got exit=%d, want %d (case-insensitive block)", variant, code, ExitBlock)
		}
	}
}

// TestRunHook_MalformedJSON_AllowsWithLog confirms graceful fallback:
// rather than break the agent on bad harness input, log + allow.
func TestRunHook_MalformedJSON_AllowsWithLog(t *testing.T) {
	withAgentID(t, "clive")
	stdin := strings.NewReader("not-valid-json")
	var stderr bytes.Buffer
	code := RunHook(stdin, &stderr)
	if code != ExitAllow {
		t.Errorf("got exit=%d, want %d (allow on decode error)", code, ExitAllow)
	}
	if !strings.Contains(stderr.String(), "clivewriteguard: decode error") {
		t.Errorf("stderr missing diagnostic on decode error; got: %s", stderr.String())
	}
}

// TestRunHook_TruncatesLongCommand verifies multi-KB Bash heredocs
// are truncated rather than dumped verbatim into stderr.
func TestRunHook_TruncatesLongCommand(t *testing.T) {
	withAgentID(t, "clive")
	longCmd := strings.Repeat("a", 500)
	stdin := strings.NewReader(jsonInput("Bash", "", longCmd))
	var stderr bytes.Buffer
	code := RunHook(stdin, &stderr)
	if code != ExitBlock {
		t.Fatalf("got exit=%d, want %d", code, ExitBlock)
	}
	if !strings.Contains(stderr.String(), "…") {
		t.Errorf("stderr should contain truncation marker for 500-char cmd; got: %s", stderr.String())
	}
	if strings.Count(stderr.String(), "a") > 250 {
		t.Errorf("stderr contains too many 'a' chars (truncate failed); got len=%d", len(stderr.String()))
	}
}
