package emma

import (
	"strings"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// TestSubprocessNewSessionArgsInjectsAgentIDEnvFlag locks the OUTBOUND-MISS
// attribution contract (mirrors brian/rain). Without `-e BOT_HQ_AGENT_ID=emma`,
// hooks fire anonymously.
func TestSubprocessNewSessionArgsInjectsAgentIDEnvFlag(t *testing.T) {
	e := &Subprocess{tmuxSession: "test-session", workDir: "/tmp"}
	args := e.newSessionArgs()

	want := "BOT_HQ_AGENT_ID=" + agentID
	found := false
	for i := 0; i < len(args)-1; i++ {
		if args[i] == "-e" && args[i+1] == want {
			found = true
			break
		}
	}
	if !found {
		t.Errorf("newSessionArgs missing `-e %s` env-injection; got %v", want, args)
	}

	if !strings.Contains(strings.Join(args, " "), "test-session") {
		t.Errorf("session name not in args: %v", args)
	}
}

// TestSubprocessInitialPromptAnchorsRole locks emma's hub-orchestrator
// persona substrings against silent drift away from vision.md.
func TestSubprocessInitialPromptAnchorsRole(t *testing.T) {
	e := &Subprocess{}
	prompt := e.InitialPromptForTest()

	cases := []struct {
		needle string
		why    string
	}{
		{"hub orchestrator", "vision.md role anchor"},
		{"DeepSeek-V4-Pro", "Z-9d model anchor"},
		{"DO NOT participate in BRAIN-cycle", "boundary anchor (no BRAIN-cycle)"},
		{"DO NOT hold state", "boundary anchor (stateless)"},
		{"DO NOT elevate", "boundary anchor (no [HR]/Flag)"},
		{"Context Library", "CL terminology anchor"},
		{"OUTPUT CONTRACT", "Z-9e-followup: emma must route every reply via hub_send"},
		{"the user received silence", "OUTPUT CONTRACT consequence framing — silence is the failure mode"},
	}
	for _, c := range cases {
		if !strings.Contains(prompt, c.needle) {
			t.Errorf("prompt missing %q (%s)", c.needle, c.why)
		}
	}
}

// TestShouldForwardToEmma covers the routing filter: emma sees broadcasts
// + cross-session elevations passing PassesMainHubView, AND only ones
// directed/mentioned to her. Self-emits drop. Session-scoped non-elevated
// chatter never reaches emma.
func TestShouldForwardToEmma(t *testing.T) {
	cases := []struct {
		name string
		msg  protocol.Message
		want bool
	}{
		{
			name: "self_emit_dropped",
			msg:  protocol.Message{FromAgent: agentID, ToAgent: "", Content: "Emma online"},
			want: false,
		},
		{
			name: "directed_to_emma_broadcast_scope_forwards",
			msg:  protocol.Message{FromAgent: "user", ToAgent: agentID, Content: "what's the plan"},
			want: true,
		},
		{
			name: "mention_emma_in_broadcast_forwards",
			msg:  protocol.Message{FromAgent: "brian", ToAgent: "", Content: "@emma can you look at vision?"},
			want: true,
		},
		{
			name: "untargeted_chatter_no_mention_dropped",
			msg:  protocol.Message{FromAgent: "brian", ToAgent: "", Content: "ack"},
			want: false,
		},
		{
			name: "session_scoped_non_elevated_dropped",
			msg:  protocol.Message{FromAgent: "brian", ToAgent: "", SessionID: "abc-123", Content: "@emma local question"},
			want: false,
		},
		{
			name: "session_scoped_elevation_forwards",
			msg:  protocol.Message{FromAgent: "brian", ToAgent: "", SessionID: "abc-123", Type: protocol.MsgFlag, Content: "@emma flag-worthy"},
			want: true,
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := shouldForwardToEmma(tc.msg)
			if got != tc.want {
				t.Errorf("shouldForwardToEmma(%+v) = %v, want %v", tc.msg, got, tc.want)
			}
		})
	}
}

// TestFormatEmmaNudge locks the nudge-format contract: MsgFlag + [HR]
// content render without sender attribution per Phase R R2.
func TestFormatEmmaNudge(t *testing.T) {
	cases := []struct {
		name string
		msg  protocol.Message
		want string
	}{
		{
			name: "regular_broadcast",
			msg:  protocol.Message{FromAgent: "user", Content: "hi emma"},
			want: "[HUB:user] hi emma",
		},
		{
			name: "msg_flag_strips_sender",
			msg:  protocol.Message{FromAgent: "rain", Type: protocol.MsgFlag, Content: "halt: foo"},
			want: "[HUB:FLAG] halt: foo",
		},
		{
			name: "hr_prefix_strips_sender",
			msg:  protocol.Message{FromAgent: "brian", Content: "[HR] please review"},
			want: "[HUB] [HR] please review",
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := formatEmmaNudge(tc.msg)
			if got != tc.want {
				t.Errorf("formatEmmaNudge = %q, want %q", got, tc.want)
			}
		})
	}
}
