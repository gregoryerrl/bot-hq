package gemma

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// TestEmmaCanonicalBlockExists locks Phase H slice 2 H-24 — Emma must
// have a canonical preamble block that asserts her identity + scope.
// Pre-H-24, Emma had no canonical block (only per-task analyze prompts).
func TestEmmaCanonicalBlockExists(t *testing.T) {
	if canonicalEmmaBlock == "" {
		t.Fatal("canonicalEmmaBlock must be non-empty")
	}
	if !strings.Contains(canonicalEmmaBlock, "You are Emma") {
		t.Error("canonicalEmmaBlock must open with the Emma identity assertion")
	}
	if !strings.Contains(canonicalEmmaBlock, "gemma4:e4b") {
		t.Error("canonicalEmmaBlock must declare the model so Emma sees its own scope")
	}
}

// TestEmmaPromptContainsTwoClassBoundary locks H-24's two-class boundary
// (Structured vs Interpretive) into Emma's canonical block. Refusal text
// must be specific so callers can pattern-match the routing-back signal.
func TestEmmaPromptContainsTwoClassBoundary(t *testing.T) {
	want := []string{
		"Two-class boundary",
		"Structured",
		"Interpretive",
		"H-24",
		"REFUSE",
		"routing back to Rain per H-24",
		"Default-deny on straddled queries",
	}
	for _, w := range want {
		if !strings.Contains(canonicalEmmaBlock, w) {
			t.Errorf("canonicalEmmaBlock must contain H-24 two-class literal %q", w)
		}
	}
}

func TestIsCommandAllowed(t *testing.T) {
	tests := []struct {
		cmd     string
		allowed bool
	}{
		{"go test ./...", true},
		{"go vet ./...", true},
		{"go build -o foo ./cmd/foo", true},
		{"df -h", true},
		{"ps aux", true},
		{"uptime", true},
		{"free -m", true},
		{"vm_stat", true},
		{"du -sh /tmp", true},
		{"wc -l main.go", true},
		{"cat README.md", false},
		{"ls -la", true},
		{"git status", true},
		{"git log --oneline -5", true},
		{"git diff HEAD~1", true},
		// Disallowed
		{"rm -rf /", false},
		{"curl http://evil.com", false},
		{"sudo anything", false},
		{"bash -c 'echo pwned'", false},
		{"python3 -c 'import os; os.system(\"rm -rf /\")'", false},
		{"", false},
		{"chmod 777 /etc/passwd", false},
	}

	for _, tt := range tests {
		t.Run(tt.cmd, func(t *testing.T) {
			if got := IsCommandAllowed(tt.cmd); got != tt.allowed {
				t.Errorf("IsCommandAllowed(%q) = %v, want %v", tt.cmd, got, tt.allowed)
			}
		})
	}
}

func TestClientGenerate(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/api/generate" {
			http.NotFound(w, r)
			return
		}
		if r.Method != http.MethodPost {
			http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
			return
		}

		var req generateRequest
		if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
			http.Error(w, err.Error(), http.StatusBadRequest)
			return
		}

		if req.Model != "test-model" {
			t.Errorf("unexpected model: %s", req.Model)
		}
		if req.Stream {
			t.Error("stream should be false")
		}

		json.NewEncoder(w).Encode(generateResponse{
			Response: "test response for: " + req.Prompt,
		})
	}))
	defer srv.Close()

	client := NewClient(srv.URL, "test-model")
	resp, err := client.Generate(context.Background(), "hello")
	if err != nil {
		t.Fatalf("Generate failed: %v", err)
	}
	if resp != "test response for: hello" {
		t.Errorf("unexpected response: %s", resp)
	}
}

func TestClientGenerateError(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Error(w, "model not found", http.StatusNotFound)
	}))
	defer srv.Close()

	client := NewClient(srv.URL, "missing-model")
	_, err := client.Generate(context.Background(), "hello")
	if err == nil {
		t.Fatal("expected error for 404 response")
	}
}

func TestClientIsHealthy(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/api/tags" {
			w.WriteHeader(http.StatusOK)
			w.Write([]byte(`{"models":[]}`))
			return
		}
		http.NotFound(w, r)
	}))
	defer srv.Close()

	client := NewClient(srv.URL, "test")
	if !client.IsHealthy(context.Background()) {
		t.Error("expected healthy")
	}

	// Test unhealthy (server down)
	srv.Close()
	client2 := NewClient(srv.URL, "test")
	if client2.IsHealthy(context.Background()) {
		t.Error("expected unhealthy after server close")
	}
}

func TestIsPseudoMount(t *testing.T) {
	tests := []struct {
		mount  string
		pseudo bool
	}{
		{"/dev", true},
		{"/dev/disk1s1", true},
		{"/proc", true},
		{"/proc/sys", true},
		{"/sys", true},
		{"/run", true},
		{"/System/Volumes/VM", true},
		{"/System/Volumes/Preboot", true},
		{"/System/Volumes/Update", true},
		{"/private/var/vm", true},
		// Real mounts must NOT be filtered.
		{"/", false},
		{"/Users", false},
		{"/Volumes/External", false},
		{"/System/Volumes/Data", false},
		{"/home", false},
		// Edge: prefix-but-not-component must not match.
		{"/develop", false},
		{"/sysadmin", false},
	}
	for _, tt := range tests {
		t.Run(tt.mount, func(t *testing.T) {
			if got := isPseudoMount(tt.mount); got != tt.pseudo {
				t.Errorf("isPseudoMount(%q) = %v, want %v", tt.mount, got, tt.pseudo)
			}
		})
	}
}

func TestShouldFlag_HysteresisDedupes(t *testing.T) {
	g := &Gemma{flagHistory: make(map[string]time.Time)}
	now := time.Now()

	if !g.shouldFlag("disk:/", now) {
		t.Fatal("first fire of a fresh condition must succeed")
	}
	if g.shouldFlag("disk:/", now.Add(5*time.Minute)) {
		t.Fatal("re-firing within hysteresis window must be suppressed")
	}
	if g.shouldFlag("disk:/", now.Add(29*time.Minute)) {
		t.Fatal("re-firing at 29min must still be suppressed (window=30m)")
	}
	if !g.shouldFlag("disk:/", now.Add(31*time.Minute)) {
		t.Fatal("re-firing past hysteresis window must succeed")
	}
}

func TestShouldFlag_RateCapAcrossConditions(t *testing.T) {
	g := &Gemma{flagHistory: make(map[string]time.Time)}
	now := time.Now()

	// Three distinct conditions inside 1h fill the cap.
	if !g.shouldFlag("a", now) {
		t.Fatal("flag 1 must succeed")
	}
	if !g.shouldFlag("b", now.Add(time.Minute)) {
		t.Fatal("flag 2 must succeed")
	}
	if !g.shouldFlag("c", now.Add(2*time.Minute)) {
		t.Fatal("flag 3 must succeed")
	}
	// Fourth distinct condition still inside the 1h window — blocked by cap.
	if g.shouldFlag("d", now.Add(3*time.Minute)) {
		t.Fatal("flag 4 within 1h window must be capped")
	}

	// After the window slides past the first three, capacity returns.
	if !g.shouldFlag("d", now.Add(61*time.Minute)) {
		t.Fatal("flag past 1h window must succeed once older fires age out")
	}
}

func TestShouldFlag_WindowPrunes(t *testing.T) {
	g := &Gemma{flagHistory: make(map[string]time.Time)}
	now := time.Now()

	// Pre-seed three old fires (>1h ago); they must be pruned.
	for i, k := range []string{"a", "b", "c"} {
		_ = i
		g.flagHistory[k] = now.Add(-2 * time.Hour)
		g.flagWindow = append(g.flagWindow, now.Add(-2*time.Hour))
	}
	if !g.shouldFlag("d", now) {
		t.Fatal("aged-out window entries must not block fresh fires")
	}
	if got := len(g.flagWindow); got != 1 {
		t.Errorf("expected window pruned to 1 entry, got %d", got)
	}
}

// Ratchet against the Emma anomaly-routing regression: monitor reports
// MUST go to Rain (EYES owns Emma), not Brian. A future refactor that
// flips this back to "brian" puts anomaly noise on the wrong agent
// and breaks the EYES role boundary.
func TestRunHealthChecksRoutesToRain(t *testing.T) {
	data, err := os.ReadFile("gemma.go")
	if err != nil {
		t.Fatal(err)
	}
	src := string(data)
	want := `ToAgent:   "rain",`
	if !strings.Contains(src, want) {
		t.Errorf("gemma.go must contain %q — anomaly reports route to Rain (EYES)", want)
	}
	if strings.Contains(src, `ToAgent:   "brian",`) {
		t.Errorf("gemma.go must not route anomalies to Brian — that violates EYES/HANDS split")
	}
}

// TestHeartbeatRefreshesLastSeen locks the per-tick contract of the
// heartbeat: calling db.UpdateAgentLastSeen(emmaID) must advance Emma's
// last_seen so panestate.ComputeActivity returns ActivityOnline (not
// Stale) when queried within HeartbeatOnlineWindow. The goroutine wiring is
// covered by the source-level check below.
func TestHeartbeatRefreshesLastSeen(t *testing.T) {
	dbPath := filepath.Join(t.TempDir(), "test.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()

	if err := db.RegisterAgent(protocol.Agent{
		ID:     agentID,
		Name:   agentName,
		Type:   protocol.AgentGemma,
		Status: protocol.StatusOnline,
	}); err != nil {
		t.Fatal(err)
	}
	a, _ := db.GetAgent(agentID)
	before := a.LastSeen

	// Force a non-zero gap so the heartbeat update is detectable even on
	// hosts where time.Now().UnixMilli() resolution might collide between
	// adjacent calls.
	time.Sleep(2 * time.Millisecond)

	if err := db.UpdateAgentLastSeen(agentID); err != nil {
		t.Fatalf("heartbeat tick failed: %v", err)
	}
	a2, _ := db.GetAgent(agentID)
	after := a2.LastSeen

	if !after.After(before) {
		t.Errorf("last_seen did not advance: before=%v after=%v", before, after)
	}
}

// TestSentinelMatchPositive locks that representative pre-filter strings
// produce Match=true and (where appropriate) AlwaysFlag=true.
func TestSentinelMatchPositive(t *testing.T) {
	cases := []struct {
		name       string
		content    string
		alwaysFlag bool
	}{
		{"panic-colon", "panic: runtime error: index out of range", true},
		{"panic-paren", "panic({0xc000..., 0xc000...})", true},
		{"deadlock-bang", "fatal error: all goroutines are asleep - deadlock!", true},
		{"rate-limit", "anthropic API rate limit exceeded", true},
		{"process-exit", "coder agent process exited with code 1", true},
		{"schema-constraint-violation", "schema constraint violation on agents.id", true},
		{"schema-constraint-failed", "schema constraint failed during migration", true},
		{"schema-constraint-error", "schema constraint error: agents.id not unique", true},
		{"sigsegv", "SIGSEGV: segmentation violation", true},
		{"fatal-no-flag", "FATAL: connection lost", false},   // pre-filter only, not always-flag
		{"oom-no-flag", "out of memory: killed worker", false}, // pre-filter only
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			d := SentinelMatch(protocol.Message{Content: tc.content})
			if !d.Match {
				t.Fatalf("expected Match=true for %q, got %+v", tc.content, d)
			}
			if d.AlwaysFlag != tc.alwaysFlag {
				t.Errorf("AlwaysFlag mismatch for %q: got %v, want %v", tc.content, d.AlwaysFlag, tc.alwaysFlag)
			}
			if d.Pattern == "" {
				t.Errorf("Pattern should be non-empty for matched message")
			}
		})
	}
}

// TestSentinelMatchNegative locks default-ignore: routine messages must
// produce Match=false so they're silently dropped instead of triggering
// observations or flags.
func TestSentinelMatchNegative(t *testing.T) {
	cases := []string{
		"Brian online. Ready for commands.",
		"Concur shape, opening worktree.",
		"Pull request merged successfully",
		"task complete: 5 files changed",
		"hello world",
		"",
		// F2 false-positive class: panic/deadlock as nouns in planning prose.
		// Post-tighten patterns require canonical Go runtime delimiters
		// (`panic:` / `panic(` / `deadlock!`) — bare-word, period-followed,
		// space-followed, and `=`-followed cases all drop out.
		"flow on panic.",
		"resolved a deadlock.",
		"on panic = retry, on deadlock = abort.",
		"on panic state",
		"deadlock condition exists",
		"discussion on panic in code review",
		// F2 false-positive class: schema constraint discussed conceptually
		// without the violation/failed/error follow-on word.
		"discussing schema constraint design",
		"the schema constraint is conservative",
		"constraint violation in our locked spec",
	}
	for _, content := range cases {
		t.Run(content, func(t *testing.T) {
			d := SentinelMatch(protocol.Message{Content: content})
			if d.Match {
				t.Errorf("expected default-ignore for %q, got Match=%v Pattern=%q", content, d.Match, d.Pattern)
			}
			if d.AlwaysFlag {
				t.Errorf("AlwaysFlag must be false when Match=false")
			}
		})
	}
}

// TestSentinelAlwaysFlagSubsetOfPreFilter is a structural ratchet: every
// always-flag pattern source must also exist verbatim in the pre-filter
// list (alwaysFlag is a strict subset). Without this invariant, a future
// editor could add an always-flag pattern that fails the pre-filter
// gate and never fires.
func TestSentinelAlwaysFlagSubsetOfPreFilter(t *testing.T) {
	pre := map[string]bool{}
	for _, p := range preFilterPatterns {
		pre[p.String()] = true
	}
	for _, p := range alwaysFlagPatterns {
		if !pre[p.String()] {
			t.Errorf("always-flag pattern %q is not in preFilterPatterns — strict subset invariant broken", p.String())
		}
	}
}

// TestOnHubMessageSkipsSelfAndDefaultIgnores locks two contracts:
//  1. Emma's own messages do not feed back into the sentinel
//  2. Non-matching messages produce no DB writes (default-ignore)
func TestOnHubMessageSkipsSelfAndDefaultIgnores(t *testing.T) {
	dbPath := filepath.Join(t.TempDir(), "test.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()

	g := New(db, hub.GemmaConfig{})

	// Self-message with a panic-like body: must NOT generate any new row.
	g.OnHubMessage(protocol.Message{FromAgent: agentID, Content: "panic: self test"})
	// Routine non-matching message: must NOT generate any new row.
	g.OnHubMessage(protocol.Message{FromAgent: "brian", Content: "concur shape, opening worktree"})

	msgs, err := db.GetRecentMessages(10)
	if err != nil {
		t.Fatal(err)
	}
	if len(msgs) != 0 {
		t.Errorf("expected zero rows after skip-self + default-ignore, got %d (%v)", len(msgs), msgs)
	}
}

// TestOnHubMessageEmitsFlagOnAlwaysFlagPattern locks the always-flag
// dispatch: a non-self message containing a panic produces a flag
// message in the DB.
func TestOnHubMessageEmitsFlagOnAlwaysFlagPattern(t *testing.T) {
	dbPath := filepath.Join(t.TempDir(), "test.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()

	g := New(db, hub.GemmaConfig{})

	g.OnHubMessage(protocol.Message{FromAgent: "brian", Content: "PANIC: nil pointer in handler"})

	msgs, err := db.GetRecentMessages(10)
	if err != nil {
		t.Fatal(err)
	}
	var flag *protocol.Message
	for i := range msgs {
		if msgs[i].FromAgent == agentID && msgs[i].Type == protocol.MsgFlag {
			flag = &msgs[i]
			break
		}
	}
	if flag == nil {
		t.Fatalf("expected MsgFlag from emma, got messages: %v", msgs)
	}
	if !strings.Contains(flag.Content, "panic") && !strings.Contains(flag.Content, "PANIC") {
		t.Errorf("flag content should reference matched panic, got %q", flag.Content)
	}
}

// TestStartWiresHeartbeatLoop locks that gemma.Start() launches the
// heartbeat goroutine. Source-level check (mirrors the existing
// TestRunHealthChecksRoutesToRain pattern) — full goroutine integration
// would require an Ollama dependency.
func TestStartWiresHeartbeatLoop(t *testing.T) {
	data, err := os.ReadFile("gemma.go")
	if err != nil {
		t.Fatal(err)
	}
	src := string(data)
	if !strings.Contains(src, "go g.heartbeatLoop()") {
		t.Errorf("gemma.go Start() must contain `go g.heartbeatLoop()` — heartbeat goroutine not wired")
	}
	if !strings.Contains(src, "func (g *Gemma) heartbeatLoop()") {
		t.Errorf("gemma.go must define heartbeatLoop on *Gemma")
	}
	if !strings.Contains(src, "UpdateAgentLastSeen(agentID)") {
		t.Errorf("heartbeatLoop must call db.UpdateAgentLastSeen(agentID)")
	}
}

// TestAgentImbalanceExcludesCoders locks the F3 whitelist contract: coder
// agents are spawn-and-die by design, so their offline rows must not count
// toward the offline-ratio anomaly. The "20 offline coders + 6 online
// non-coders" steady-state should produce no flag.
func TestAgentImbalanceExcludesCoders(t *testing.T) {
	agents := []protocol.Agent{
		{ID: "brian", Type: protocol.AgentBrian, Status: protocol.StatusOnline},
		{ID: "rain", Type: protocol.AgentQA, Status: protocol.StatusOnline},
		{ID: "emma", Type: protocol.AgentGemma, Status: protocol.StatusOnline},
		{ID: "discord", Type: protocol.AgentDiscord, Status: protocol.StatusOnline},
		{ID: "clive", Type: protocol.AgentVoice, Status: protocol.StatusOnline},
		{ID: "live", Type: protocol.AgentVoice, Status: protocol.StatusOnline},
	}
	for i := 0; i < 20; i++ {
		agents = append(agents, protocol.Agent{
			ID:     fmt.Sprintf("coder-%d", i),
			Type:   protocol.AgentCoder,
			Status: protocol.StatusOffline,
		})
	}
	if a, ok := checkAgentImbalance(agents); ok {
		t.Errorf("expected no anomaly (6 non-coder online + 20 offline coders excluded), got %+v", a)
	}
}

// TestAgentImbalanceFiresOnNonCoders locks the inverse: when non-coder
// agents skew offline, the anomaly still fires. This is the F3 ratchet —
// the coder whitelist must not accidentally suppress real imbalance signal.
func TestAgentImbalanceFiresOnNonCoders(t *testing.T) {
	agents := []protocol.Agent{
		{ID: "brian", Type: protocol.AgentBrian, Status: protocol.StatusOffline},
		{ID: "rain", Type: protocol.AgentQA, Status: protocol.StatusOffline},
		{ID: "emma", Type: protocol.AgentGemma, Status: protocol.StatusOnline},
		{ID: "discord", Type: protocol.AgentDiscord, Status: protocol.StatusOffline},
		// One coder included — must not change the outcome.
		{ID: "coder-1", Type: protocol.AgentCoder, Status: protocol.StatusOnline},
	}
	a, ok := checkAgentImbalance(agents)
	if !ok {
		t.Fatalf("expected anomaly (3 non-coder offline vs 1 non-coder online), got none")
	}
	if a.key != "agent-imbalance" {
		t.Errorf("expected key=agent-imbalance, got %q", a.key)
	}
	if !strings.Contains(a.msg, "1 online, 3 offline") {
		t.Errorf("anomaly msg should report non-coder counts (1 online, 3 offline), got %q", a.msg)
	}
}

// TestAgentImbalanceCoderOnlyNoFlag locks the edge case: when only coder
// agents exist (no non-coder rows), nothing is anomalous regardless of
// their status distribution. The whitelist should not crash or false-fire.
func TestAgentImbalanceCoderOnlyNoFlag(t *testing.T) {
	agents := []protocol.Agent{
		{ID: "coder-1", Type: protocol.AgentCoder, Status: protocol.StatusOffline},
		{ID: "coder-2", Type: protocol.AgentCoder, Status: protocol.StatusOffline},
		{ID: "coder-3", Type: protocol.AgentCoder, Status: protocol.StatusOnline},
	}
	if a, ok := checkAgentImbalance(agents); ok {
		t.Errorf("expected no anomaly (coder-only roster), got %+v", a)
	}
}
