package emma

import (
	"strings"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// TestEmmaPromptContainsUserDirective locks the user msg 15734
// directive verbatim cite into emma's prompt. Phase S S-1b cite-
// anchor lock per R18.
func TestEmmaPromptContainsUserDirective(t *testing.T) {
	e := &Emma{}
	prompt := e.initialPrompt()
	want := []string{
		"USER DIRECTIVE (msg 15734)",
		"emma to be the enforcer of the trio",
		"watching you guys",
	}
	for _, s := range want {
		if !strings.Contains(prompt, s) {
			t.Errorf("emma prompt must cite user directive substring %q", s)
		}
	}
}

// TestEmmaPromptContainsEYESClass locks the EYES-class read-only
// boundary into the prompt + tool-restriction substrings.
func TestEmmaPromptContainsEYESClass(t *testing.T) {
	e := &Emma{}
	prompt := e.initialPrompt()
	want := []string{
		"EYES-class read-only enforcer",
		"You NEVER edit code",
		"Edit / Write / Bash",
		"You NEVER inject system-reminders",
	}
	for _, s := range want {
		if !strings.Contains(prompt, s) {
			t.Errorf("emma prompt must contain EYES-class substring %q", s)
		}
	}
}

// TestEmmaPromptContainsSpeechTrigger verifies the silent-unless
// gating directives.
func TestEmmaPromptContainsSpeechTrigger(t *testing.T) {
	e := &Emma{}
	prompt := e.initialPrompt()
	want := []string{
		"SPEECH-TRIGGER",
		"silent unless",
		"@emma",
		"Rule-violation observed",
	}
	for _, s := range want {
		if !strings.Contains(prompt, s) {
			t.Errorf("emma prompt must contain speech-trigger substring %q", s)
		}
	}
}

// TestEmmaPromptContainsScopePriorBaseline locks the 8-item narrative-
// class scope-prior baseline (A) per Phase S S-1b user-confirm.
func TestEmmaPromptContainsScopePriorBaseline(t *testing.T) {
	e := &Emma{}
	prompt := e.initialPrompt()
	want := []string{
		"SCOPE-PRIOR (A) GUIDED-DISCRETION BASELINE",
		"NARRATIVE-CLASS violations",
		"parking / heartbeat-loop antipattern",
		"Non-continuation-after-user-directive",
		"Cross-timing-dedup misuse",
		"Handshake-terminator misuse",
		"SCOPE-FORK-CONFIRMATION skipped",
		"FILESYSTEM-SIGNAL-CITE skipped",
		"R37 ESTIMATE-SHAPE-DISCLOSURE skipped",
		"SNAP-GATING violations",
		"DISCRETION CLAUSE",
		"rules-are-absolute",
	}
	for _, s := range want {
		if !strings.Contains(prompt, s) {
			t.Errorf("emma prompt must contain scope-prior baseline substring %q", s)
		}
	}
}

// TestEmmaPromptContainsR20Bootstrap locks R20 BOOTSTRAP parity per
// Rain msg 15835 disposition.
func TestEmmaPromptContainsR20Bootstrap(t *testing.T) {
	e := &Emma{}
	prompt := e.initialPrompt()
	want := []string{
		"R20 BOOTSTRAP-ON-CONVERSATION-RESUME",
		"~/.bot-hq/emma/last_state.json",
		"AgentState",
	}
	for _, s := range want {
		if !strings.Contains(prompt, s) {
			t.Errorf("emma prompt must contain R20 bootstrap substring %q", s)
		}
	}
}

// TestEmmaPromptContainsOutputChannel locks hub_send-broadcast-only
// output discipline + no-system-reminder-pane-injection rule.
func TestEmmaPromptContainsOutputChannel(t *testing.T) {
	e := &Emma{}
	prompt := e.initialPrompt()
	want := []string{
		"OUTPUT CHANNEL: hub_send broadcast",
		"NEVER inject system-reminders",
	}
	for _, s := range want {
		if !strings.Contains(prompt, s) {
			t.Errorf("emma prompt must contain output-channel substring %q", s)
		}
	}
}

// TestShouldForwardToEmma_SelfSkipped prevents feedback loops by
// skipping emma's own messages from forward.
func TestShouldForwardToEmma_SelfSkipped(t *testing.T) {
	if shouldForwardToEmma(protocol.Message{FromAgent: "emma", Content: "self"}) {
		t.Error("emma's own messages should be skipped from forward (feedback-loop prevention)")
	}
}

// TestShouldForwardToEmma_PeerForwarded — emma watches all peer
// traffic for rule-violations. Brian / rain / discord / coder all
// forward.
func TestShouldForwardToEmma_PeerForwarded(t *testing.T) {
	cases := []string{"brian", "rain", "discord", "coder-abc", "user", "gemma"}
	for _, from := range cases {
		t.Run(from, func(t *testing.T) {
			if !shouldForwardToEmma(protocol.Message{FromAgent: from, Content: "peer"}) {
				t.Errorf("peer agent %q should forward to emma", from)
			}
		})
	}
}

// TestFormatNudge_Broadcast renders broadcast (no ToAgent).
func TestFormatNudge_Broadcast(t *testing.T) {
	got := formatNudge(protocol.Message{FromAgent: "brian", Content: "concur"})
	if !strings.HasPrefix(got, "[HUB:brian]") {
		t.Errorf("broadcast nudge should start with [HUB:brian]; got %q", got)
	}
}

// TestFormatNudge_Directed renders directed (ToAgent set).
func TestFormatNudge_Directed(t *testing.T) {
	got := formatNudge(protocol.Message{FromAgent: "rain", ToAgent: "brian", Content: "PASS-2"})
	if !strings.HasPrefix(got, "[HUB:rain→brian]") {
		t.Errorf("directed nudge should include →target; got %q", got)
	}
}

// TestFormatNudge_FlagBroadcast renders MsgFlag broadcast.
func TestFormatNudge_FlagBroadcast(t *testing.T) {
	got := formatNudge(protocol.Message{FromAgent: "brian", Type: protocol.MsgFlag, Content: "alert"})
	if !strings.HasPrefix(got, "[HUB:FLAG:brian]") {
		t.Errorf("flag broadcast nudge should start with [HUB:FLAG:brian]; got %q", got)
	}
}

// TestFormatNudge_FlagDirected renders MsgFlag directed.
func TestFormatNudge_FlagDirected(t *testing.T) {
	got := formatNudge(protocol.Message{FromAgent: "rain", ToAgent: "user", Type: protocol.MsgFlag, Content: "halt"})
	if !strings.HasPrefix(got, "[HUB:FLAG:rain→user]") {
		t.Errorf("flag directed nudge should include →target; got %q", got)
	}
}

// TestFormatNudge_PreservesAttributionForViolationDetection — emma
// needs FROM-AGENT verbatim to detect violations. Unlike brian/rain
// which strip on [HR]/FLAG per R2, emma's render keeps attribution.
func TestFormatNudge_PreservesAttributionForViolationDetection(t *testing.T) {
	got := formatNudge(protocol.Message{FromAgent: "rain", Content: "[HR] BRAIN-final-seal"})
	if !strings.Contains(got, "rain") {
		t.Errorf("emma formatNudge should preserve from-agent attribution (audit-load-bearing); got %q", got)
	}
}

// TestNew_HasDefaults locks New constructor's default-fill behavior.
func TestNew_HasDefaults(t *testing.T) {
	e := New(nil, "")
	if e.workDir == "" {
		t.Error("workDir should default-fill when empty arg passed")
	}
	if e.stopCh == nil {
		t.Error("stopCh should be initialized")
	}
}

// TestIsRunning_Initial checks initial state.
func TestIsRunning_Initial(t *testing.T) {
	e := New(nil, "")
	if e.IsRunning() {
		t.Error("emma should not be running pre-Start")
	}
}

// TestNewSessionArgs_InjectsAgentID — BOT_HQ_AGENT_ID env-var must
// be injected for outbound-hook agent attribution.
func TestNewSessionArgs_InjectsAgentID(t *testing.T) {
	e := &Emma{tmuxSession: "test-sess", workDir: "/tmp"}
	args := e.newSessionArgs()
	found := false
	for _, a := range args {
		if a == "BOT_HQ_AGENT_ID=emma" {
			found = true
			break
		}
	}
	if !found {
		t.Errorf("newSessionArgs should inject BOT_HQ_AGENT_ID=emma; got %v", args)
	}
}
