package brian

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

func setupTestDB(t *testing.T) *hub.DB {
	t.Helper()
	path := filepath.Join(t.TempDir(), "test.db")
	db, err := hub.OpenDB(path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	return db
}

func TestNudgeContainsMessageContent(t *testing.T) {
	content := "fix the login bug"
	nudge := formatNudge(protocol.Message{FromAgent: "user", Content: content})

	if !strings.Contains(nudge, content) {
		t.Errorf("nudge should contain message content %q, got: %s", content, nudge)
	}
	if !strings.Contains(nudge, "user") {
		t.Errorf("nudge should contain sender 'user', got: %s", nudge)
	}
}

func TestFormatNudgeIsNotEmpty(t *testing.T) {
	nudge := formatNudge(protocol.Message{FromAgent: "user", Content: "hello"})
	if nudge == "" {
		t.Error("formatNudge should return non-empty string")
	}
	if !strings.Contains(nudge, "hello") {
		t.Error("formatNudge should include content")
	}
	if !strings.Contains(nudge, "[HUB:user]") {
		t.Error("formatNudge should include sender in [HUB:<sender>] tag")
	}
}

func TestInitialPromptMentionsHandshake(t *testing.T) {
	b := &Brian{}
	prompt := b.initialPrompt()
	if !strings.Contains(prompt, "handshake") {
		t.Error("initial prompt should mention handshake protocol")
	}
	if !strings.Contains(prompt, "hub_session_create") {
		t.Error("initial prompt should mention hub_session_create")
	}
}

// Ratchet against regression: the OUTBOUND contract must survive any future
// prompt compression. If this line goes missing, replies regress to tmux-only
// and the user sees silence (see 2026-04-24 incident).
func TestInitialPromptContainsOutboundContract(t *testing.T) {
	b := &Brian{}
	prompt := b.initialPrompt()
	want := "OUTBOUND: every reply is a hub_send tool call."
	if !strings.Contains(prompt, want) {
		t.Errorf("initial prompt must contain OUTBOUND contract substring %q", want)
	}
	if !strings.Contains(prompt, "you did not answer") {
		t.Error("initial prompt must keep the self-check clause ('you did not answer')")
	}
}

// Ratchet against regression: the prompt must embed the canonical
// DiscV2OutboundRule const verbatim. If the inline OUTBOUND text drifts
// back into brian.go (or the const reference is dropped), the agent loses
// the audience-driven routing rule and reverts to the older private-default
// behavior that was half of the 2026-04-24 peer-visibility incident.
//
// The const itself is ratchet-tested in protocol/disc_test.go; this test
// locks the wiring on the brian end.
func TestInitialPromptEmbedsDiscV2OutboundRule(t *testing.T) {
	b := &Brian{}
	prompt := b.initialPrompt()
	if !strings.Contains(prompt, protocol.DiscV2OutboundRule) {
		t.Errorf("initial prompt must embed protocol.DiscV2OutboundRule verbatim (bug #1 wiring lock)")
	}
}

// Ratchet against the cliff-hang failure mode observed at msg 2086-2092
// on 2026-04-25: scope changes within an ongoing decision require a
// fresh flag, not silent continuation. The old "1 concern = 1 flag"
// wording let us read scope-morphs as "still on the same flag" and
// hold quietly while the user watched a silent hub. DISC v2.1 reframes
// from per-concern accounting to per-state — every pending-on-user
// state gets a flag once entering it, including refinements that
// materially alter the pending shape.
func TestInitialPromptContainsDISCv21FlagRule(t *testing.T) {
	b := &Brian{}
	prompt := b.initialPrompt()
	// Phase H slice 2 H-2: FLAG governance shifted from symmetric ALWAYS-FLAG
	// to asymmetric Rain-owns-elevation. Substrings ratchet the new shape;
	// the old "DECISION POINT / Per-state, not per-concern" framing was
	// rewritten in C1.
	want := []string{
		"Rain owns elevation",
		"Brian PMs Rain on flag-worthy events",
		"scope changes mid-decision",
		"cliff-hang",
	}
	for _, w := range want {
		if !strings.Contains(prompt, w) {
			t.Errorf("initial prompt must contain DISC v2.2 FLAG literal %q", w)
		}
	}
}

// TestBrianPromptContainsHalterPusher locks the Phase H slice 2 H-1
// halter/pusher ratchet into Brian's initial prompt. Asymmetric "Rain halts,
// Brian pushes through" mechanic prevents mutual-halt deadlock.
func TestBrianPromptContainsHalterPusher(t *testing.T) {
	b := &Brian{}
	prompt := b.initialPrompt()
	want := []string{
		"HALTER/PUSHER",
		"Rain halts, Brian pushes through",
		"BRAIN-cycle exempt",
		"Mutual-halt deadlock impossible by construction",
	}
	for _, w := range want {
		if !strings.Contains(prompt, w) {
			t.Errorf("initial prompt must contain H-1 halter/pusher literal %q", w)
		}
	}
}

// TestBrianPromptContainsCarveOutEnumeration locks the Phase H slice 2 H-2
// self-flag carve-out enumeration. Brian only self-flags in enumerated
// catastrophe cases when Rain is unreachable; otherwise PMs Rain.
func TestBrianPromptContainsCarveOutEnumeration(t *testing.T) {
	b := &Brian{}
	prompt := b.initialPrompt()
	want := []string{
		"push-failure",
		"repo-corruption",
		"auth-failure",
		"hub-disconnect",
		"git-state-unexpected-on-write-path",
		"Rain unreachable >60s",
		"[self-flag-carve-out: <reason>]",
	}
	for _, w := range want {
		if !strings.Contains(prompt, w) {
			t.Errorf("initial prompt must contain H-2 self-flag carve-out literal %q", w)
		}
	}
}

// TestBrianPromptContainsGreenflagDelegation locks the 2026-04-27 user
// greenflag delegation: Rain may pick joint defaults without flagging when
// user is not in the loop on the specific decision.
func TestBrianPromptContainsGreenflagDelegation(t *testing.T) {
	b := &Brian{}
	prompt := b.initialPrompt()
	want := []string{
		"2026-04-27 user delegation",
		"greenflag authority",
		"Rain may pick joint defaults without flag",
	}
	for _, w := range want {
		if !strings.Contains(prompt, w) {
			t.Errorf("initial prompt must contain greenflag delegation literal %q", w)
		}
	}
}

// TestBrianPromptContainsAsymmetricPivot locks the H-2 consistency fold:
// PIVOT scenario routes through Rain's hub_flag elevation, not Brian's
// self-flag. Closes the symmetric-flag residue Rain caught in C1 review.
func TestBrianPromptContainsAsymmetricPivot(t *testing.T) {
	b := &Brian{}
	prompt := b.initialPrompt()
	want := []string{
		"PIVOT: user w/o executor",
		"Brian PMs Rain",
		"Rain holds 60s",
		"elevates via hub_flag",
	}
	for _, w := range want {
		if !strings.Contains(prompt, w) {
			t.Errorf("initial prompt must contain H-2 asymmetric PIVOT literal %q", w)
		}
	}
	// And the OLD symmetric framing must be GONE.
	for _, gone := range []string{"brian flags, rain holds 60s"} {
		if strings.Contains(prompt, gone) {
			t.Errorf("initial prompt must NOT contain old symmetric PIVOT literal %q", gone)
		}
	}
}

// TestBrianStartupBootstrapIterate locks the H-19 caller-side iterate
// discipline into STARTUP step 1. Without iteration, large backlogs
// (post-rebuild, post-idle) silently truncate at 50; the agent acts on
// a partial mental model. Substrings ratchet the iterate-pattern + the
// convention-doc pointer.
func TestBrianStartupBootstrapIterate(t *testing.T) {
	b := &Brian{}
	prompt := b.initialPrompt()
	want := []string{
		"iterate with `since_id = last_msg.id`",
		"empty batch",
		"hub_read caps at 50",
		"docs/conventions/bootstrap-iterate.md",
	}
	for _, w := range want {
		if !strings.Contains(prompt, w) {
			t.Errorf("initial prompt must contain H-19 bootstrap-iterate literal %q", w)
		}
	}
}

// TestBrianPromptContainsHaltAllWork locks the H-31 halt-all-work convention
// into the initial prompt. The literal substrings ratchet the substring
// recognition pattern (slice 4 C7 M1 fold per Rain msg 3820: rephrased from
// regex-anchor notation to "contains substring" framing since agents are
// LLM-interpreters and semantic-match the pattern; regex-anchor notation
// could mislead future-Brian into thinking literal pattern-match was
// authoritative), the close-via-H15 directive, and the fresh-session
// restart contract.
func TestBrianPromptContainsHaltAllWork(t *testing.T) {
	b := &Brian{}
	prompt := b.initialPrompt()
	want := []string{
		"HALT-ALL-WORK (H-31, H-33)",
		`"agent <id> at <N>%, halt"`,
		`"plan usage at <N>%, halt"`,
		"Match by substring meaning across BOTH triggers (agent context-cap OR plan-usage), not regex anchors",
		"Both fire HALT-ALL-WORK",
		"hub_session_close",
		"fresh-context session",
		"H-15 ledger pre-loads context",
	}
	for _, w := range want {
		if !strings.Contains(prompt, w) {
			t.Errorf("initial prompt must contain HALT-ALL-WORK literal %q", w)
		}
	}
}

// TestBrianStartupCarveOutGloss locks the H-2 consistency fold for STARTUP:
// Brian's startup-time hub_flag is explicitly framed as the implicit
// carve-out window (Rain not yet registered).
func TestBrianStartupCarveOutGloss(t *testing.T) {
	b := &Brian{}
	prompt := b.initialPrompt()
	want := []string{
		"startup carve-out: Rain not yet registered, self-flag is implicit per H-2",
	}
	for _, w := range want {
		if !strings.Contains(prompt, w) {
			t.Errorf("initial prompt must contain STARTUP carve-out gloss %q", w)
		}
	}
}

// Ratchet against regression: DISC v2 role split (HANDS/EYES/BRAIN) + OUTPUT
// class rules must survive future prompt compression. Each literal is
// load-bearing — missing any of these silently re-opens a drift mode we
// already diagnosed (2026-04-24). Failure is mechanical: restore the literal.
func TestInitialPromptContainsDISCv2(t *testing.T) {
	b := &Brian{}
	prompt := b.initialPrompt()
	want := []string{
		"HANDS (brian):",
		"EYES (rain):",
		"BRAIN (both):",
		"Neither rubber-stamps; silence = implicit approval.",
		"Class-split suspended.",
		"Cannot expand Emma's allowlist",
		"EYES is read-only",
		"Rain cannot edit code",
		"OUTPUT: user replies split by class",
	}
	for _, w := range want {
		if !strings.Contains(prompt, w) {
			t.Errorf("initial prompt must contain DISC v2 literal %q", w)
		}
	}
}

// TestInitialPromptContainsH13ForcePushProtocol locks Phase H slice 1
// H-13 force-push token verification protocol into Brian's initial prompt.
// Brian is the authority that relays the user's verbatim token; failure
// to embed = coders cannot escalate force-push requests safely.
func TestInitialPromptContainsH13ForcePushProtocol(t *testing.T) {
	b := &Brian{}
	prompt := b.initialPrompt()
	if !strings.Contains(prompt, protocol.H13ForcePushProtocol) {
		t.Errorf("initial prompt must embed protocol.H13ForcePushProtocol verbatim (Phase H slice 1 wiring lock)")
	}
	// Spot-check the load-bearing literals so a const drift that drops the
	// gate without removing the constant reference still fails CI.
	want := []string{
		"H-13 FORCE-PUSH TOKEN PROTOCOL",
		"request_force_push: <branch>@<sha>",
		"force-push-greenlight: <branch>@<sha>",
		"Never bypass",
	}
	for _, w := range want {
		if !strings.Contains(prompt, w) {
			t.Errorf("H-13 protocol must contain literal %q", w)
		}
	}
}

func TestFormatNudgeCompactTagAndNoTrailer(t *testing.T) {
	nudge := formatNudge(protocol.Message{FromAgent: "user", Content: "hello"})
	if nudge != "[HUB:user] hello" {
		t.Errorf("expected compact tag, got %q", nudge)
	}
	// The IMPORTANT trailer has been moved to the initial-prompt DISCIPLINE/NUDGE
	// contract. It must not appear per-message any more — that's the compression win.
	if strings.Contains(nudge, "IMPORTANT") {
		t.Error("formatNudge should not contain the IMPORTANT trailer (moved to initial prompt)")
	}
	if strings.Contains(nudge, "hub_send") {
		t.Error("formatNudge should not contain routing instructions (hub_send)")
	}
}

func TestFormatNudgeFlagVariant(t *testing.T) {
	nudge := formatNudge(protocol.Message{FromAgent: "rain", Type: protocol.MsgFlag, Content: "disagree on scope"})
	if nudge != "[HUB:FLAG:rain] disagree on scope" {
		t.Errorf("expected broadcast FLAG tag, got %q", nudge)
	}
}

// Ratchet against regression: nudge tags must distinguish directed (PM) from
// broadcast (HUB) routing so Brian can tell at a glance whether he's the sole
// recipient or one of many. Missing PM variant silently reverts to [HUB:X]
// for direct sends — the exact confusion surfaced by the 2026-04-24 incident.
func TestFormatNudgePMAndHubVariants(t *testing.T) {
	cases := []struct {
		name string
		msg  protocol.Message
		want string
	}{
		{"PM from rain", protocol.Message{FromAgent: "rain", ToAgent: "brian", Type: protocol.MsgResponse, Content: "private"}, "[PM:rain] private"},
		{"PM from user", protocol.Message{FromAgent: "user", ToAgent: "brian", Type: protocol.MsgCommand, Content: "do x"}, "[PM:user] do x"},
		{"PM from discord", protocol.Message{FromAgent: "discord", ToAgent: "brian", Type: protocol.MsgResponse, Content: "hi"}, "[PM:discord] hi"},
		{"PM from coder", protocol.Message{FromAgent: "7a776ee2", ToAgent: "brian", Type: protocol.MsgResult, Content: "done"}, "[PM:7a776ee2] done"},
		{"PM FLAG from rain", protocol.Message{FromAgent: "rain", ToAgent: "brian", Type: protocol.MsgFlag, Content: "stop"}, "[PM:FLAG:rain] stop"},
		{"HUB broadcast from rain", protocol.Message{FromAgent: "rain", ToAgent: "", Type: protocol.MsgResponse, Content: "broad"}, "[HUB:rain] broad"},
		{"HUB broadcast from user", protocol.Message{FromAgent: "user", ToAgent: "", Type: protocol.MsgCommand, Content: "all"}, "[HUB:user] all"},
		{"HUB FLAG broadcast", protocol.Message{FromAgent: "rain", ToAgent: "", Type: protocol.MsgFlag, Content: "bug"}, "[HUB:FLAG:rain] bug"},
		{"HUB-OBS cross-traffic", protocol.Message{FromAgent: "rain", ToAgent: "user", Type: protocol.MsgResponse, Content: "reply"}, "[HUB-OBS:rain→user] reply"},
		{"HUB-OBS to discord", protocol.Message{FromAgent: "rain", ToAgent: "discord", Type: protocol.MsgResponse, Content: "post"}, "[HUB-OBS:rain→discord] post"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := formatNudge(tc.msg); got != tc.want {
				t.Errorf("formatNudge = %q, want %q", got, tc.want)
			}
		})
	}
}

// Ratchet against regression: initial prompt must document the PM/HUB/HUB-OBS
// tag split so the agent knows which tag means which routing.
func TestInitialPromptDocumentsPMTag(t *testing.T) {
	b := &Brian{}
	prompt := b.initialPrompt()
	for _, literal := range []string{"[PM:<sender>]", "[HUB:<sender>]", "[HUB-OBS:<from>→<to>]"} {
		if !strings.Contains(prompt, literal) {
			t.Errorf("initial prompt must document tag %q", literal)
		}
	}
}

// Ratchet against regression: Brian must see peer replies to user/discord in
// real time. Without this escape Brian is blind to Rain's to="user" replies
// (2026-04-24 incident: parallel drafts, neither agent saw the other's send).
// Mirror of rain.go:319-325 escape.
func TestShouldForwardToBrian_PeerToUserVisibility(t *testing.T) {
	cases := []struct {
		name string
		msg  protocol.Message
		want bool
	}{
		{"rain to user forwards", protocol.Message{FromAgent: "rain", ToAgent: "user", Type: protocol.MsgResponse, Content: "x"}, true},
		{"rain to discord forwards", protocol.Message{FromAgent: "rain", ToAgent: "discord", Type: protocol.MsgResponse, Content: "x"}, true},
		{"user to rain forwards (visible coordination)", protocol.Message{FromAgent: "user", ToAgent: "rain", Type: protocol.MsgCommand, Content: "x"}, true},
		{"discord to rain forwards", protocol.Message{FromAgent: "discord", ToAgent: "rain", Type: protocol.MsgResponse, Content: "x"}, true},
		{"user broadcast forwards", protocol.Message{FromAgent: "user", ToAgent: "", Type: protocol.MsgCommand, Content: "x"}, true},
		{"rain to brian forwards", protocol.Message{FromAgent: "rain", ToAgent: "brian", Type: protocol.MsgResponse, Content: "x"}, true},
		{"broadcast forwards", protocol.Message{FromAgent: "rain", ToAgent: "", Type: protocol.MsgResponse, Content: "x"}, true},
		{"rain to emma skips (peer-to-coder chatter)", protocol.Message{FromAgent: "rain", ToAgent: "emma", Type: protocol.MsgResponse, Content: "x"}, false},
		{"coder to coder skips", protocol.Message{FromAgent: "6058b444", ToAgent: "b4e5593f", Type: protocol.MsgUpdate, Content: "x"}, false},
		{"own message skipped", protocol.Message{FromAgent: "brian", ToAgent: "user", Type: protocol.MsgResponse, Content: "x"}, false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := shouldForwardToBrian(tc.msg); got != tc.want {
				t.Errorf("shouldForwardToBrian(%+v) = %v, want %v", tc.msg, got, tc.want)
			}
		})
	}
}

// TestStartInitDoesNotPreSeedLastMsgID is a source ratchet locking the C2
// deletion: brian.go must NOT pre-seed lastMsgID to the highest existing ID at
// init. The pre-fix init block called GetRecentMessages(1) and assigned
// msgs[0].ID to b.lastMsgID, which silently skipped any pre-restart backlog.
// First poll-tick now relies on ReadMessages tail semantics (sinceID=0 →
// latest N) to replay recent context.
func TestStartInitDoesNotPreSeedLastMsgID(t *testing.T) {
	data, err := os.ReadFile("brian.go")
	if err != nil {
		t.Fatal(err)
	}
	src := string(data)
	for _, banned := range []string{
		"b.db.GetRecentMessages(1)",
		"b.lastMsgID = msgs[0].ID",
	} {
		if strings.Contains(src, banned) {
			t.Errorf("brian.go must not contain %q — reintroduces the pre-restart backlog skip bug", banned)
		}
	}
}

// TestProcessNewMessagesAdvancesWatermark seeds N+5 messages in a fresh DB,
// constructs a Brian with zero-valued lastMsgID, calls processNewMessages
// once, and verifies the watermark advances to the highest seen ID. A second
// call returns nothing (no spurious replay — locks polling stability and the
// C1+C2 interaction).
func TestProcessNewMessagesAdvancesWatermark(t *testing.T) {
	db := setupTestDB(t)

	const seedCount = 55 // > the 50-row limit so we exercise the cap path
	var maxID int64
	for i := 0; i < seedCount; i++ {
		id, err := db.InsertMessage(protocol.Message{
			FromAgent: "user",
			Type:      protocol.MsgCommand,
			Content:   "msg",
		})
		if err != nil {
			t.Fatal(err)
		}
		if id > maxID {
			maxID = id
		}
	}

	b := New(db, t.TempDir())
	if b.lastMsgID != 0 {
		t.Fatalf("New(): lastMsgID = %d, want 0 (Go zero-value)", b.lastMsgID)
	}

	// First call: sinceID=0 path → ReadMessages tail returns latest 50.
	// SendCommand will fail (no tmux), but the watermark advances regardless.
	b.processNewMessages()
	if b.lastMsgID != maxID {
		t.Errorf("after first poll: lastMsgID = %d, want %d (latest seeded ID)", b.lastMsgID, maxID)
	}

	// Second call: sinceID=maxID path → ReadMessages returns empty, no advance.
	prev := b.lastMsgID
	b.processNewMessages()
	if b.lastMsgID != prev {
		t.Errorf("after second poll: lastMsgID = %d, want %d (no spurious replay)", b.lastMsgID, prev)
	}
}

// Regression-lock for the autostart env-var injection. The Stop hook in
// internal/outboundhook/hook.go:88 reads BOT_HQ_AGENT_ID to attribute
// OUTBOUND-MISS sentinel events to a specific agent. Without the -e flag,
// hooks fire anonymously. See msg 4197/4205 for the failure-mode framing.
func TestNewSessionArgsInjectsAgentIDEnvFlag(t *testing.T) {
	b := &Brian{tmuxSession: "test-session", workDir: "/tmp"}
	args := b.newSessionArgs()

	want := "BOT_HQ_AGENT_ID=" + agentID
	found := false
	for i := 0; i < len(args)-1; i++ {
		if args[i] == "-e" && args[i+1] == want {
			found = true
			break
		}
	}
	if !found {
		t.Errorf("newSessionArgs missing `-e %s` env-injection flag pair; got %v", want, args)
	}

	if !strings.Contains(strings.Join(args, " "), "test-session") {
		t.Errorf("session name not in args: %v", args)
	}
}
