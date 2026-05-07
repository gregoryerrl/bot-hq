package toolgate

import (
	"bytes"
	"strings"
	"testing"
)

func TestVerifyHubBroadcastRainGate_AllowRain(t *testing.T) {
	allow, reason := VerifyHubBroadcastRainGate("rain")
	if !allow {
		t.Errorf("rain should be allowed; reason=%q", reason)
	}
	if reason != "" {
		t.Errorf("rain allow path should have empty reason; got %q", reason)
	}
}

func TestVerifyHubBroadcastRainGate_BlockBrian(t *testing.T) {
	allow, reason := VerifyHubBroadcastRainGate("brian")
	if allow {
		t.Error("brian should be BLOCKED on hub_broadcast")
	}
	if !strings.Contains(reason, "Rain-only gate") {
		t.Errorf("expected 'Rain-only gate' in reason, got %q", reason)
	}
}

func TestVerifyHubBroadcastRainGate_AllowEmptyAgentID(t *testing.T) {
	// Defensive default: non-trio Claude Code instances (no BOT_HQ_AGENT_ID
	// env-var set) should be allowed through. Mirrors K-16 + R33 pattern.
	allow, _ := VerifyHubBroadcastRainGate("")
	if !allow {
		t.Error("empty agent_id should be allowed (defensive default for non-trio sessions)")
	}
}

func TestRunHook_HubBroadcastBlocksBrian(t *testing.T) {
	t.Setenv("BOT_HQ_AGENT_ID", "brian")
	stdin := strings.NewReader(`{"tool_name":"mcp__bot-hq__hub_broadcast","tool_input":{"content":"[HR] foo","from":"brian"}}`)
	var stderr bytes.Buffer
	exitCode := RunHook(stdin, &stderr)
	if exitCode != ExitBlock {
		t.Errorf("expected ExitBlock, got %d", exitCode)
	}
	if !strings.Contains(stderr.String(), "Rain-only gate") {
		t.Errorf("expected stderr to mention 'Rain-only gate'; got %q", stderr.String())
	}
}

func TestRunHook_HubBroadcastAllowsRain(t *testing.T) {
	t.Setenv("BOT_HQ_AGENT_ID", "rain")
	stdin := strings.NewReader(`{"tool_name":"mcp__bot-hq__hub_broadcast","tool_input":{"content":"[HR] foo","from":"rain"}}`)
	var stderr bytes.Buffer
	exitCode := RunHook(stdin, &stderr)
	if exitCode != ExitAllow {
		t.Errorf("expected ExitAllow for rain, got %d (stderr=%q)", exitCode, stderr.String())
	}
}

func TestHubBroadcastToolNameLiteralMatch(t *testing.T) {
	// Per Refine-5: literal-string match (not glob/regex). Test that
	// the const matches the canonical MCP tool name.
	const want = "mcp__bot-hq__hub_broadcast"
	if HubBroadcastToolName != want {
		t.Errorf("HubBroadcastToolName = %q, want %q (literal match per Refine-5)", HubBroadcastToolName, want)
	}
}
