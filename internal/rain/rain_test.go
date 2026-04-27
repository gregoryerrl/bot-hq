package rain

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

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

// 1. TestNew_DefaultWorkDir — New(db, "") → workDir should be ~/Projects
func TestNew_DefaultWorkDir(t *testing.T) {
	db := setupTestDB(t)
	r := New(db, "")

	home, err := os.UserHomeDir()
	if err != nil {
		// If UserHomeDir fails, fallback is os.TempDir()
		home = os.TempDir()
	}
	expected := filepath.Join(home, "Projects")

	if r.workDir != expected {
		t.Errorf("expected workDir %q, got %q", expected, r.workDir)
	}
}

// 2. TestNew_CustomWorkDir — New(db, "/tmp/foo") → workDir = "/tmp/foo"
func TestNew_CustomWorkDir(t *testing.T) {
	db := setupTestDB(t)
	r := New(db, "/tmp/foo")

	if r.workDir != "/tmp/foo" {
		t.Errorf("expected workDir %q, got %q", "/tmp/foo", r.workDir)
	}
}

// 3. TestNew_FieldsInitialized — stopCh not nil, running=false, lastMsgID=0
func TestNew_FieldsInitialized(t *testing.T) {
	db := setupTestDB(t)
	r := New(db, "/tmp/test")

	if r.stopCh == nil {
		t.Error("expected stopCh to be non-nil")
	}
	if r.running {
		t.Error("expected running to be false")
	}
	if r.lastMsgID != 0 {
		t.Errorf("expected lastMsgID 0, got %d", r.lastMsgID)
	}
}

// 4. TestIsRunning_Default — false on fresh instance
func TestIsRunning_Default(t *testing.T) {
	db := setupTestDB(t)
	r := New(db, "/tmp/test")

	if r.IsRunning() {
		t.Error("expected IsRunning() to return false on fresh instance")
	}
}

// 5. TestStop_NotRunning_NoOp — call Stop() on fresh instance, no panic
func TestStop_NotRunning_NoOp(t *testing.T) {
	db := setupTestDB(t)
	r := New(db, "/tmp/test")

	// Should not panic
	r.Stop()

	if r.IsRunning() {
		t.Error("expected IsRunning() to return false after Stop() on fresh instance")
	}
}

// 6. TestFormatRainNudge_BasicFormat — compact [HUB:<sender>] tag, no IMPORTANT trailer.
func TestFormatRainNudge_BasicFormat(t *testing.T) {
	result := formatRainNudge(protocol.Message{FromAgent: "brian", Content: "Please review the code"})

	if result != "[HUB:brian] Please review the code" {
		t.Errorf("expected compact tag, got %q", result)
	}
	if strings.Contains(result, "IMPORTANT") {
		t.Error("nudge should not contain the IMPORTANT trailer (moved to initial-prompt NUDGE contract)")
	}
}

// 7. TestFormatRainNudge_EmptyContent — handles empty content without dropping the tag.
func TestFormatRainNudge_EmptyContent(t *testing.T) {
	result := formatRainNudge(protocol.Message{FromAgent: "brian", Content: ""})

	if !strings.HasPrefix(result, "[HUB:brian]") {
		t.Errorf("expected nudge to start with [HUB:brian], got %q", result)
	}
}

// 8. TestFormatRainNudge_SpecialChars — quotes, newlines, tabs survive compression.
func TestFormatRainNudge_SpecialChars(t *testing.T) {
	content := "He said \"hello\"\nand then\ttabs"
	result := formatRainNudge(protocol.Message{FromAgent: "user", Content: content})

	if !strings.Contains(result, `"hello"`) {
		t.Errorf("expected nudge to preserve quotes, got %q", result)
	}
	if !strings.Contains(result, "\n") {
		t.Errorf("expected nudge to preserve newlines, got %q", result)
	}
	if !strings.Contains(result, "\t") {
		t.Errorf("expected nudge to preserve tabs, got %q", result)
	}
}

// Ratchet against regression: the OUTBOUND contract must survive any future
// prompt compression. Rain mirrors Brian's contract (see 2026-04-24 incident).
func TestInitialPromptContainsOutboundContract(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	want := "OUTBOUND: every reply is a hub_send tool call."
	if !strings.Contains(prompt, want) {
		t.Errorf("initial prompt must contain OUTBOUND contract substring %q", want)
	}
	if !strings.Contains(prompt, "you did not answer") {
		t.Error("initial prompt must keep the self-check clause ('you did not answer')")
	}
}

// Ratchet against regression: the prompt must embed the canonical
// DiscV2OutboundRule const verbatim. Mirror of brian_test.go's
// TestInitialPromptEmbedsDiscV2OutboundRule. The const itself is
// ratchet-tested in protocol/disc_test.go; this test locks the wiring
// on the rain end. Drift in either rain.go OR brian.go is now caught
// by the per-agent wiring tests.
func TestInitialPromptEmbedsDiscV2OutboundRule(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
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
// materially alter the pending shape. Rain mirrors Brian's ratchet.
func TestInitialPromptContainsDISCv21FlagRule(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	// Phase H slice 2 H-2: FLAG governance shifted from symmetric to
	// asymmetric (Rain owns elevation). Mirrors Brian's ratchet.
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

// TestRainPromptContainsHalterPusher locks the Phase H slice 2 H-1
// halter/pusher ratchet into Rain's initial prompt. Mirrors Brian's
// ratchet — same literals, same load-bearing role.
func TestRainPromptContainsHalterPusher(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
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

// TestRainPromptContainsHaltAllWork locks the H-31 halt-all-work convention
// into Rain's initial prompt. Mirrors Brian's ratchet so both agents
// recognize Emma's context-cap FLAG identically.
func TestRainPromptContainsHaltAllWork(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	want := []string{
		"HALT-ALL-WORK (H-31)",
		`^agent .* at \d+%, halt`,
		"hub_session_close",
		"fresh-context session",
		"H-15 ledger pre-loads context",
	}
	for _, w := range want {
		if !strings.Contains(prompt, w) {
			t.Errorf("initial prompt must contain H-31 halt-all-work literal %q", w)
		}
	}
}

// TestRainPromptContainsCarveOutEnumeration locks the Phase H slice 2 H-2
// self-flag carve-out enumeration into Rain's initial prompt. Mirrors
// Brian's ratchet so both agents share the same canonical carve-out list.
func TestRainPromptContainsCarveOutEnumeration(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
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

// TestRainPromptHasNoPollingRule locks Phase H slice 2 H-18 — Rain's
// initial prompt must NOT instruct hub_read polling. Hub-push delivers
// messages automatically; polling was dead weight per session-3 evidence.
// Symmetric with Brian's prompt (brian.go:237 already correct).
func TestRainPromptHasNoPollingRule(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	// The OLD polling rule must be GONE. Match the specific retired phrasing
	// (with cadence) — not the bare token "poll hub_read", since the H-18
	// replacement guidance itself contains "do NOT poll hub_read" as the
	// intentional negative instruction.
	for _, gone := range []string{
		"poll hub_read (no agent filter) every 5-10s",
		"poll hub_read every",
	} {
		if strings.Contains(prompt, gone) {
			t.Errorf("initial prompt must NOT contain H-18 retired polling literal %q", gone)
		}
	}
	// Replacement guidance must be present.
	want := []string{
		"Messages arrive automatically",
		"do NOT poll hub_read",
	}
	for _, w := range want {
		if !strings.Contains(prompt, w) {
			t.Errorf("initial prompt must contain H-18 replacement literal %q", w)
		}
	}
}

// TestRainPromptContainsAsymmetricPivot locks the H-2 consistency fold:
// PIVOT scenario routes through Rain's hub_flag elevation, not Brian's
// self-flag. Mirrors Brian's ratchet — identical canonical text per
// single-source-of-truth pattern established for HALTER/PUSHER + FLAG.
func TestRainPromptContainsAsymmetricPivot(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
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
	for _, gone := range []string{"Brian flags first; step in if no ack"} {
		if strings.Contains(prompt, gone) {
			t.Errorf("initial prompt must NOT contain old symmetric PIVOT literal %q", gone)
		}
	}
}

// Ratchet against regression: DISC v2 role split (HANDS/EYES/BRAIN) + OUTPUT
// class rules must survive future prompt compression. Rain mirrors Brian's
// ratchet — same literals, same diagnostic load (see 2026-04-24 discussion).
func TestInitialPromptContainsDISCv2(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
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

// 8b. TestFormatRainNudge_FlagVariant — MsgFlag elevates to [HUB:FLAG:<sender>].
func TestFormatRainNudge_FlagVariant(t *testing.T) {
	result := formatRainNudge(protocol.Message{FromAgent: "brian", Type: protocol.MsgFlag, Content: "scope disagreement"})

	if result != "[HUB:FLAG:brian] scope disagreement" {
		t.Errorf("expected FLAG-prefixed tag, got %q", result)
	}
}

// 8c. TestFormatRainNudge_ObserveVariant — directed-to-other-agent becomes [HUB-OBS:<from>→<to>].
func TestFormatRainNudge_ObserveVariant(t *testing.T) {
	result := formatRainNudge(protocol.Message{FromAgent: "brian", ToAgent: "discord", Content: "posting update"})

	if result != "[HUB-OBS:brian→discord] posting update" {
		t.Errorf("expected HUB-OBS variant for inter-agent traffic, got %q", result)
	}
}

// Ratchet against regression: nudge tags must distinguish directed (PM) from
// broadcast (HUB) routing so Rain can tell at a glance whether she's the sole
// recipient or one of many. Mirror of brian_test.go TestFormatNudgePMAndHubVariants.
func TestFormatRainNudgePMAndHubVariants(t *testing.T) {
	cases := []struct {
		name string
		msg  protocol.Message
		want string
	}{
		{"PM from brian", protocol.Message{FromAgent: "brian", ToAgent: "rain", Type: protocol.MsgResponse, Content: "private"}, "[PM:brian] private"},
		{"PM from user", protocol.Message{FromAgent: "user", ToAgent: "rain", Type: protocol.MsgCommand, Content: "do x"}, "[PM:user] do x"},
		{"PM from discord", protocol.Message{FromAgent: "discord", ToAgent: "rain", Type: protocol.MsgResponse, Content: "hi"}, "[PM:discord] hi"},
		{"PM from coder", protocol.Message{FromAgent: "7a776ee2", ToAgent: "rain", Type: protocol.MsgResult, Content: "done"}, "[PM:7a776ee2] done"},
		{"PM FLAG from brian", protocol.Message{FromAgent: "brian", ToAgent: "rain", Type: protocol.MsgFlag, Content: "stop"}, "[PM:FLAG:brian] stop"},
		{"HUB broadcast from brian", protocol.Message{FromAgent: "brian", ToAgent: "", Type: protocol.MsgResponse, Content: "broad"}, "[HUB:brian] broad"},
		{"HUB broadcast from user", protocol.Message{FromAgent: "user", ToAgent: "", Type: protocol.MsgCommand, Content: "all"}, "[HUB:user] all"},
		{"HUB FLAG broadcast", protocol.Message{FromAgent: "brian", ToAgent: "", Type: protocol.MsgFlag, Content: "bug"}, "[HUB:FLAG:brian] bug"},
		{"HUB-OBS cross-traffic", protocol.Message{FromAgent: "brian", ToAgent: "user", Type: protocol.MsgResponse, Content: "reply"}, "[HUB-OBS:brian→user] reply"},
		{"HUB-OBS to discord", protocol.Message{FromAgent: "brian", ToAgent: "discord", Type: protocol.MsgResponse, Content: "post"}, "[HUB-OBS:brian→discord] post"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := formatRainNudge(tc.msg); got != tc.want {
				t.Errorf("formatRainNudge = %q, want %q", got, tc.want)
			}
		})
	}
}

// Ratchet against regression: initial prompt must document the PM/HUB/HUB-OBS
// tag split so Rain knows which tag means which routing.
func TestInitialPromptDocumentsPMTag(t *testing.T) {
	r := &Rain{}
	prompt := r.initialPrompt()
	for _, literal := range []string{"[PM:<sender>]", "[HUB:<sender>]", "[HUB-OBS:<from>→<to>]"} {
		if !strings.Contains(prompt, literal) {
			t.Errorf("initial prompt must document tag %q", literal)
		}
	}
}

// Ratchet against regression: Rain must see peer replies to user/discord in
// real time. The bug was two-layered: SQL filter at db.go:354 excluded
// cross-traffic from Rain's query, AND rain.go pollLoop used ReadMessages("rain")
// which activated that SQL filter. Fix moves filter authority fully into
// shouldForwardToRain() by calling ReadMessages("") — this test locks the
// Go-layer decisions. See 2026-04-24 incident.
func TestShouldForwardToRain_PeerToUserVisibility(t *testing.T) {
	cases := []struct {
		name string
		msg  protocol.Message
		want bool
	}{
		{"brian to user forwards", protocol.Message{FromAgent: "brian", ToAgent: "user", Type: protocol.MsgResponse, Content: "x"}, true},
		{"brian to discord forwards", protocol.Message{FromAgent: "brian", ToAgent: "discord", Type: protocol.MsgResponse, Content: "x"}, true},
		{"brian broadcast response forwards (peer visibility)", protocol.Message{FromAgent: "brian", ToAgent: "", Type: protocol.MsgResponse, Content: "x"}, true},
		{"user to brian forwards (visible coordination)", protocol.Message{FromAgent: "user", ToAgent: "brian", Type: protocol.MsgCommand, Content: "x"}, true},
		{"directed to rain forwards", protocol.Message{FromAgent: "brian", ToAgent: "rain", Type: protocol.MsgResponse, Content: "x"}, true},
		{"user broadcast forwards", protocol.Message{FromAgent: "user", ToAgent: "", Type: protocol.MsgCommand, Content: "x"}, true},
		{"coder result forwards (QA coverage)", protocol.Message{FromAgent: "7a776ee2", ToAgent: "brian", Type: protocol.MsgResult, Content: "x"}, true},
		{"flag forwards", protocol.Message{FromAgent: "brian", ToAgent: "", Type: protocol.MsgFlag, Content: "x"}, true},
		{"hub_flag mention forwards", protocol.Message{FromAgent: "emma", ToAgent: "", Type: protocol.MsgUpdate, Content: "calling hub_flag"}, true},
		{"coder broadcast response skips (no flood)", protocol.Message{FromAgent: "7a776ee2", ToAgent: "", Type: protocol.MsgResponse, Content: "ack"}, false},
		{"coder to coder skips", protocol.Message{FromAgent: "6058b444", ToAgent: "b4e5593f", Type: protocol.MsgUpdate, Content: "x"}, false},
		{"emma to brian update skips", protocol.Message{FromAgent: "emma", ToAgent: "brian", Type: protocol.MsgUpdate, Content: "x"}, false},
		{"own message skipped", protocol.Message{FromAgent: "rain", ToAgent: "user", Type: protocol.MsgResponse, Content: "x"}, false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := shouldForwardToRain(tc.msg); got != tc.want {
				t.Errorf("shouldForwardToRain(%+v) = %v, want %v", tc.msg, got, tc.want)
			}
		})
	}
}

// Ratchet against the SQL-layer half of the bug: Rain's poll MUST use
// ReadMessages("", ...) not ReadMessages("rain", ...). The agentID-scoped
// SQL query filters cross-traffic before Go ever sees it, silently
// invalidating shouldForwardToRain()'s user/discord escape clauses.
// This test asserts the source contains the unscoped call.
func TestProcessNewMessagesUsesUnscoppedRead(t *testing.T) {
	data, err := os.ReadFile("rain.go")
	if err != nil {
		t.Fatal(err)
	}
	src := string(data)
	want := `r.db.ReadMessages("", r.lastMsgID, 50)`
	if !strings.Contains(src, want) {
		t.Errorf("rain.go must call %s — scoped ReadMessages(agentID, ...) reintroduces the SQL-layer blindspot", want)
	}
	unwanted := `r.db.ReadMessages(agentID, r.lastMsgID`
	if strings.Contains(src, unwanted) {
		t.Errorf("rain.go must not call %s — that reintroduces the bug", unwanted)
	}
}

// 9. TestWriteMCPConfig_JSONStructure — create Rain with t.TempDir() workDir,
// call writeMCPConfig(), read and parse the JSON file, verify structure.
func TestWriteMCPConfig_JSONStructure(t *testing.T) {
	db := setupTestDB(t)
	tmpDir := t.TempDir()
	r := New(db, tmpDir)

	if err := r.writeMCPConfig(); err != nil {
		t.Fatal(err)
	}

	configPath := filepath.Join(tmpDir, ".bot-hq-rain-mcp.json")
	data, err := os.ReadFile(configPath)
	if err != nil {
		t.Fatalf("failed to read config file: %v", err)
	}

	var config map[string]any
	if err := json.Unmarshal(data, &config); err != nil {
		t.Fatalf("failed to parse JSON: %v", err)
	}

	mcpServers, ok := config["mcpServers"].(map[string]any)
	if !ok {
		t.Fatal("expected mcpServers key in config")
	}

	botHQ, ok := mcpServers["bot-hq"].(map[string]any)
	if !ok {
		t.Fatal("expected bot-hq key in mcpServers")
	}

	if _, ok := botHQ["command"]; !ok {
		t.Error("expected command field in bot-hq config")
	}

	args, ok := botHQ["args"]
	if !ok {
		t.Fatal("expected args field in bot-hq config")
	}

	argsList, ok := args.([]any)
	if !ok {
		t.Fatalf("expected args to be an array, got %T", args)
	}

	if len(argsList) != 1 || argsList[0] != "mcp" {
		t.Errorf("expected args=[\"mcp\"], got %v", argsList)
	}

	// Verify file permissions
	info, err := os.Stat(configPath)
	if err != nil {
		t.Fatal(err)
	}
	perm := info.Mode().Perm()
	if perm != 0600 {
		t.Errorf("expected file permission 0600, got %04o", perm)
	}
}

// 10. TestProcessNewMessages_SkipsSelf — insert a message from "rain" to "rain",
// call processNewMessages, verify no SendCommand attempt.
func TestProcessNewMessages_SkipsSelf(t *testing.T) {
	db := setupTestDB(t)
	r := New(db, t.TempDir())

	// Register rain agent so messages can be addressed to it
	db.RegisterAgent(protocol.Agent{
		ID:     "rain",
		Name:   "Rain",
		Type:   protocol.AgentQA,
		Status: protocol.StatusOnline,
	})

	// Insert a message from rain to rain
	_, err := db.InsertMessage(protocol.Message{
		FromAgent: "rain",
		ToAgent:   "rain",
		Type:      protocol.MsgUpdate,
		Content:   "self-message that should be skipped",
		Created:   time.Now(),
	})
	if err != nil {
		t.Fatal(err)
	}

	// processNewMessages should skip messages from self.
	// Since Rain is not running (no tmux), SendCommand would return an error.
	// If the skip logic works, SendCommand is never called and no error occurs.
	// We verify by checking that lastMsgID advances (message was seen) but
	// no panic/error from trying to send to a non-existent tmux session.
	r.processNewMessages()

	if r.lastMsgID == 0 {
		t.Error("expected lastMsgID to advance after processing messages")
	}
}

// TestStartInitDoesNotPreSeedLastMsgID is a source ratchet locking the C2
// deletion: rain.go must NOT pre-seed lastMsgID to the highest existing ID at
// init. The pre-fix init block called GetRecentMessages(1) and assigned
// msgs[0].ID to r.lastMsgID, which silently skipped any pre-restart backlog.
// First poll-tick now relies on ReadMessages tail semantics (sinceID=0 →
// latest N) to replay recent context.
func TestStartInitDoesNotPreSeedLastMsgID(t *testing.T) {
	data, err := os.ReadFile("rain.go")
	if err != nil {
		t.Fatal(err)
	}
	src := string(data)
	for _, banned := range []string{
		"r.db.GetRecentMessages(1)",
		"r.lastMsgID = msgs[0].ID",
	} {
		if strings.Contains(src, banned) {
			t.Errorf("rain.go must not contain %q — reintroduces the pre-restart backlog skip bug", banned)
		}
	}
}

// TestProcessNewMessagesNoSpuriousReplay locks polling stability: a second
// processNewMessages call after the watermark has advanced returns nothing.
// This is the C1+C2 interaction at the floor — verifies sinceID=lastID
// returns empty (Rain's third C1 test assertion expressed at the call site).
func TestProcessNewMessagesNoSpuriousReplay(t *testing.T) {
	db := setupTestDB(t)

	for i := 0; i < 30; i++ {
		if _, err := db.InsertMessage(protocol.Message{
			FromAgent: "user",
			Type:      protocol.MsgCommand,
			Content:   "msg",
		}); err != nil {
			t.Fatal(err)
		}
	}

	r := New(db, t.TempDir())
	r.processNewMessages()
	advanced := r.lastMsgID
	if advanced == 0 {
		t.Fatal("first poll: expected lastMsgID to advance, got 0")
	}

	r.processNewMessages()
	if r.lastMsgID != advanced {
		t.Errorf("second poll: lastMsgID = %d, want %d (no spurious replay)", r.lastMsgID, advanced)
	}
}
