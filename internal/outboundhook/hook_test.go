package outboundhook

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
)

// writeTranscript builds a Claude-Code-shaped JSONL transcript at path.
// Each event is one line. Helps tests construct realistic last-turn
// shapes (user → assistant text + tool_use → assistant text …).
func writeTranscript(t *testing.T, path string, events []map[string]any) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	var buf strings.Builder
	for _, ev := range events {
		data, err := json.Marshal(ev)
		if err != nil {
			t.Fatal(err)
		}
		buf.Write(data)
		buf.WriteByte('\n')
	}
	if err := os.WriteFile(path, []byte(buf.String()), 0o600); err != nil {
		t.Fatal(err)
	}
}

func userEvent(text string) map[string]any {
	return map[string]any{
		"type":      "user",
		"timestamp": "2026-04-28T05:00:00Z",
		"message": map[string]any{
			"role":    "user",
			"content": []any{map[string]any{"type": "text", "text": text}},
		},
	}
}

func assistantEvent(ts string, blocks ...map[string]any) map[string]any {
	content := make([]any, len(blocks))
	for i, b := range blocks {
		content[i] = b
	}
	return map[string]any{
		"type":      "assistant",
		"timestamp": ts,
		"message": map[string]any{
			"role":    "assistant",
			"content": content,
		},
	}
}

func textBlock(text string) map[string]any {
	return map[string]any{"type": "text", "text": text}
}

func toolUseBlock(name string) map[string]any {
	return map[string]any{"type": "tool_use", "name": name, "input": map[string]any{}}
}

// TestParseLastTurnAggregatesAfterLastUser locks the turn-boundary
// detection: only assistant content after the most recent user message
// counts toward the last-turn summary.
func TestParseLastTurnAggregatesAfterLastUser(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "transcript.jsonl")
	writeTranscript(t, path, []map[string]any{
		userEvent("first user"),
		assistantEvent("2026-04-28T05:00:01Z", textBlock("ignored prior turn"), toolUseBlock("mcp__bot-hq__hub_send")),
		userEvent("second user"),
		assistantEvent("2026-04-28T05:00:02Z", textBlock("DRAFT current turn"), toolUseBlock("Read")),
	})

	s, err := parseLastTurn(path)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(s.TextSnip, "DRAFT current turn") {
		t.Errorf("text snip should be from current turn, got %q", s.TextSnip)
	}
	if strings.Contains(s.TextSnip, "ignored prior turn") {
		t.Errorf("text snip leaked prior-turn content: %q", s.TextSnip)
	}
	if s.HubSent {
		t.Errorf("HubSent should be false — current turn has no hub_send (the prior turn's hub_send is excluded)")
	}
	if s.Timestamp != "2026-04-28T05:00:02Z" {
		t.Errorf("timestamp should be from current-turn assistant event, got %q", s.Timestamp)
	}
}

// TestShouldFlagThreeClause locks the filter contract:
//
//  1. text_len > 0 (excludes tool-only turns)
//  2. (planning keyword present OR text > 200 chars)
//  3. no hub_send tool call
func TestShouldFlagThreeClause(t *testing.T) {
	cases := []struct {
		name string
		s    turnSummary
		want bool
	}{
		{"empty turn", turnSummary{}, false},
		{"tool-only turn", turnSummary{TextLen: 0, HubSent: false}, false},
		{"hub_send made", turnSummary{TextLen: 500, TextSnip: strings.Repeat("a", 500), HubSent: true}, false},
		{"short non-keyword text", turnSummary{TextLen: 50, TextSnip: "ok done"}, false},
		{"keyword short text", turnSummary{TextLen: 30, TextSnip: "DRAFT one-liner"}, true},
		{"long non-keyword text", turnSummary{TextLen: 250, TextSnip: strings.Repeat("a ", 130)}, true},
		{"keyword + long + no hub_send", turnSummary{TextLen: 500, TextSnip: "concur. " + strings.Repeat("a", 500)}, true},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := shouldFlag(tc.s); got != tc.want {
				t.Errorf("shouldFlag(%+v) = %v, want %v", tc.s, got, tc.want)
			}
		})
	}
}

// TestHasPlanningKeyword spot-checks a few representative tokens.
func TestHasPlanningKeyword(t *testing.T) {
	cases := map[string]bool{
		"DRAFT-alone, Rain wait":           true,
		"BRAIN-second on Rain's draft":     true,
		"concur on the converged scope":    true,
		"pushback on framing":              true,
		"[Rain to Brian] standing by":      true,
		"SNAP captured":                    true,
		"plain status update no signals":   false,
		"tool work, ran several Read ops":  false,
	}
	for text, want := range cases {
		t.Run(text, func(t *testing.T) {
			if got := hasPlanningKeyword(text); got != want {
				t.Errorf("hasPlanningKeyword(%q) = %v, want %v", text, got, want)
			}
		})
	}
}

// TestRunHookEmitsAlertOnPositiveFilter is the slice-5 acceptance test:
// fake transcript with planning text and no hub_send, hook subcommand
// invocation, verify hub.DB has the OUTBOUND-MISS alert from the
// configured agent.
func TestRunHookEmitsAlertOnPositiveFilter(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("BOT_HQ_HOME", dir)
	t.Setenv(agentIDEnvVar, "rain")
	dbPath := filepath.Join(dir, "test.db")
	t.Setenv(dbPathEnvVar, dbPath)

	// Pre-create the DB so the hook can open it.
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	db.Close()

	transcriptPath := filepath.Join(dir, "transcript.jsonl")
	writeTranscript(t, transcriptPath, []map[string]any{
		userEvent("brainstorm question"),
		assistantEvent("2026-04-28T05:10:00Z",
			textBlock("DRAFT-alone, Rain wait. Pushback on framing — fix root cause first."),
			toolUseBlock("Read"),
		),
	})

	hookInput := map[string]any{
		"session_id":      "test-sess",
		"transcript_path": transcriptPath,
		"hook_event_name": "Stop",
	}
	data, err := json.Marshal(hookInput)
	if err != nil {
		t.Fatal(err)
	}
	if err := RunHook(strings.NewReader(string(data))); err != nil {
		t.Fatal(err)
	}

	db2, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer db2.Close()
	msgs, err := db2.GetRecentMessages(10)
	if err != nil {
		t.Fatal(err)
	}
	found := false
	for _, m := range msgs {
		if m.FromAgent == "rain" && strings.Contains(m.Content, "OUTBOUND-MISS") {
			found = true
		}
	}
	if !found {
		t.Errorf("expected OUTBOUND-MISS alert from rain in hub DB, got messages: %v", msgs)
	}
}

// TestRunHookSkipsOnNegativeFilter locks the no-FP contract: a turn
// with hub_send made, even with planning keywords, must NOT emit.
func TestRunHookSkipsOnNegativeFilter(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("BOT_HQ_HOME", dir)
	t.Setenv(agentIDEnvVar, "brian")
	dbPath := filepath.Join(dir, "test.db")
	t.Setenv(dbPathEnvVar, dbPath)

	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	db.Close()

	transcriptPath := filepath.Join(dir, "transcript.jsonl")
	writeTranscript(t, transcriptPath, []map[string]any{
		userEvent("question"),
		assistantEvent("2026-04-28T05:11:00Z",
			textBlock("DRAFT-alone — concur."),
			toolUseBlock("mcp__bot-hq__hub_send"),
		),
	})

	hookInput := map[string]any{"transcript_path": transcriptPath}
	data, _ := json.Marshal(hookInput)
	RunHook(strings.NewReader(string(data)))

	db2, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer db2.Close()
	msgs, _ := db2.GetRecentMessages(10)
	for _, m := range msgs {
		if strings.Contains(m.Content, "OUTBOUND-MISS") {
			t.Errorf("filter should suppress turn that did make hub_send; got OUTBOUND-MISS: %v", m)
		}
	}
}

// TestRunHookSkipsWhenAgentIDUnset locks the safety carve-out: hook
// installed in a non-bot-hq claude session must silently no-op rather
// than spamming alerts. agentIDEnvVar absent ⇒ no emit.
func TestRunHookSkipsWhenAgentIDUnset(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("BOT_HQ_HOME", dir)
	t.Setenv(agentIDEnvVar, "")
	dbPath := filepath.Join(dir, "test.db")
	t.Setenv(dbPathEnvVar, dbPath)

	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	db.Close()

	transcriptPath := filepath.Join(dir, "transcript.jsonl")
	writeTranscript(t, transcriptPath, []map[string]any{
		userEvent("question"),
		assistantEvent("2026-04-28T05:12:00Z", textBlock("DRAFT response")),
	})

	hookInput := map[string]any{"transcript_path": transcriptPath}
	data, _ := json.Marshal(hookInput)
	RunHook(strings.NewReader(string(data)))

	db2, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer db2.Close()
	msgs, _ := db2.GetRecentMessages(10)
	if len(msgs) > 0 {
		t.Errorf("hook should no-op without agent_id env; got %d messages", len(msgs))
	}
}

// TestRunHookDedupSuppressesRepeat locks idempotency: two invocations
// of the same turn within dedupWindow must produce a single alert.
func TestRunHookDedupSuppressesRepeat(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("BOT_HQ_HOME", dir)
	t.Setenv(agentIDEnvVar, "rain")
	dbPath := filepath.Join(dir, "test.db")
	t.Setenv(dbPathEnvVar, dbPath)

	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	db.Close()

	transcriptPath := filepath.Join(dir, "transcript.jsonl")
	writeTranscript(t, transcriptPath, []map[string]any{
		userEvent("question"),
		assistantEvent("2026-04-28T05:13:00Z", textBlock("DRAFT response stable identifier")),
	})

	hookInput := map[string]any{"transcript_path": transcriptPath}
	data, _ := json.Marshal(hookInput)

	RunHook(strings.NewReader(string(data)))
	RunHook(strings.NewReader(string(data)))
	RunHook(strings.NewReader(string(data)))

	db2, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer db2.Close()
	msgs, _ := db2.GetRecentMessages(10)
	count := 0
	for _, m := range msgs {
		if strings.Contains(m.Content, "OUTBOUND-MISS") {
			count++
		}
	}
	if count != 1 {
		t.Errorf("dedup window should collapse 3 invocations to 1 alert; got %d", count)
	}
}

// TestInstallTrioHookFreshFile locks: missing settings.json → created
// with our hook entry only.
func TestInstallTrioHookFreshFile(t *testing.T) {
	dir := t.TempDir()
	settingsPath := filepath.Join(dir, ".claude", "settings.json")
	if err := InstallTrioHook(settingsPath, "/usr/local/bin/bot-hq"); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(settingsPath)
	if err != nil {
		t.Fatalf("settings.json not created: %v", err)
	}
	var s map[string]any
	if err := json.Unmarshal(data, &s); err != nil {
		t.Fatalf("settings.json not valid JSON: %v", err)
	}
	hooks, _ := s["hooks"].(map[string]any)
	stop, _ := hooks["Stop"].([]any)
	if len(stop) != 1 {
		t.Fatalf("expected 1 Stop hook entry, got %d: %v", len(stop), stop)
	}
}

// TestInstallTrioHookIdempotent locks: running twice must not duplicate
// the entry.
func TestInstallTrioHookIdempotent(t *testing.T) {
	dir := t.TempDir()
	settingsPath := filepath.Join(dir, "settings.json")
	if err := InstallTrioHook(settingsPath, "/usr/local/bin/bot-hq"); err != nil {
		t.Fatal(err)
	}
	if err := InstallTrioHook(settingsPath, "/usr/local/bin/bot-hq"); err != nil {
		t.Fatal(err)
	}
	data, _ := os.ReadFile(settingsPath)
	var s map[string]any
	json.Unmarshal(data, &s)
	hooks, _ := s["hooks"].(map[string]any)
	stop, _ := hooks["Stop"].([]any)
	if len(stop) != 1 {
		t.Errorf("idempotency violation: 2 installs produced %d entries", len(stop))
	}
}

// TestInstallTrioHookPreservesUnrelatedHook locks: existing user hooks
// (other matchers, other event types) must be preserved verbatim.
func TestInstallTrioHookPreservesUnrelatedHook(t *testing.T) {
	dir := t.TempDir()
	settingsPath := filepath.Join(dir, "settings.json")
	existing := map[string]any{
		"hooks": map[string]any{
			"PreToolUse": []any{
				map[string]any{
					"matcher": "Bash",
					"hooks": []any{
						map[string]any{"type": "command", "command": "echo my-pre-tool-hook"},
					},
				},
			},
			"Stop": []any{
				map[string]any{
					"matcher": "",
					"hooks": []any{
						map[string]any{"type": "command", "command": "echo my-existing-stop-hook"},
					},
				},
			},
		},
	}
	data, _ := json.MarshalIndent(existing, "", "  ")
	os.WriteFile(settingsPath, data, 0o600)

	if err := InstallTrioHook(settingsPath, "/usr/local/bin/bot-hq"); err != nil {
		t.Fatal(err)
	}
	final, _ := os.ReadFile(settingsPath)
	var s map[string]any
	json.Unmarshal(final, &s)
	hooks, _ := s["hooks"].(map[string]any)

	// PreToolUse hook must survive untouched.
	pre, _ := hooks["PreToolUse"].([]any)
	if len(pre) != 1 {
		t.Fatalf("PreToolUse clobbered, got %v", pre)
	}

	// Stop array should now have 2 entries: existing user one + ours.
	stop, _ := hooks["Stop"].([]any)
	if len(stop) != 2 {
		t.Fatalf("Stop should have 2 entries (preserved + added), got %d: %v", len(stop), stop)
	}
	preserved := false
	added := false
	for _, e := range stop {
		em := e.(map[string]any)
		inner := em["hooks"].([]any)
		for _, h := range inner {
			cmd := h.(map[string]any)["command"].(string)
			if cmd == "echo my-existing-stop-hook" {
				preserved = true
			}
			if strings.Contains(cmd, "outbound-miss-hook") {
				added = true
			}
		}
	}
	if !preserved {
		t.Errorf("user's existing Stop hook clobbered")
	}
	if !added {
		t.Errorf("our Stop hook not appended")
	}
}

// TestInstallTrioHookInvalidJSONReturnsError locks the no-silent-corrupt
// contract: invalid JSON → error, no rewrite.
func TestInstallTrioHookInvalidJSONReturnsError(t *testing.T) {
	dir := t.TempDir()
	settingsPath := filepath.Join(dir, "settings.json")
	os.WriteFile(settingsPath, []byte("{ this is not json"), 0o600)

	err := InstallTrioHook(settingsPath, "/usr/local/bin/bot-hq")
	if err == nil {
		t.Fatal("expected error on invalid JSON, got nil")
	}
	if !strings.Contains(err.Error(), "parse") && !strings.Contains(err.Error(), "JSON") {
		t.Errorf("error should mention parse/JSON, got: %v", err)
	}
	// Source file unchanged.
	data, _ := os.ReadFile(settingsPath)
	if string(data) != "{ this is not json" {
		t.Errorf("invalid settings.json should not be rewritten, got %q", data)
	}
}

// TestSettingsHookCommandShape locks the wire format embedded into
// settings.json so it stays parseable by Claude Code's hooks runner.
func TestSettingsHookCommandShape(t *testing.T) {
	got := SettingsHookCommand("/usr/local/bin/bot-hq")
	want := "/usr/local/bin/bot-hq outbound-miss-hook"
	if got != want {
		t.Errorf("SettingsHookCommand = %q, want %q", got, want)
	}
}

// TestDedupKeyStable locks that the dedup key is deterministic across
// processes — same inputs → same key, allowing cross-invocation dedup.
func TestDedupKeyStable(t *testing.T) {
	a := dedupKey("/tmp/transcript.jsonl", "2026-04-28T05:00:00Z")
	b := dedupKey("/tmp/transcript.jsonl", "2026-04-28T05:00:00Z")
	if a != b {
		t.Errorf("dedupKey not deterministic: %q vs %q", a, b)
	}
	c := dedupKey("/tmp/transcript.jsonl", "2026-04-28T05:00:01Z")
	if a == c {
		t.Errorf("different turn timestamps must produce different keys")
	}
}

// TestDedupWindowExpiry verifies that an old entry past dedupWindow
// does NOT suppress a new emit. Avoids permanent suppression of a
// recurring discipline gap.
func TestDedupWindowExpiry(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("BOT_HQ_HOME", dir)

	now := time.Now()
	old := now.Add(-2 * dedupWindow)
	recordDedup("/tmp/transcript.jsonl", "2026-04-28T05:00:00Z", old)

	if alreadyFlaggedRecently("/tmp/transcript.jsonl", "2026-04-28T05:00:00Z", now) {
		t.Errorf("expired ledger entry should not suppress new emit")
	}
}
