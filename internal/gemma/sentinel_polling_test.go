package gemma

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// TestPollSentinelCatchesCrossProcessInserts is the slice-2 hotfix
// load-bearing integration test. It models the cross-process MCP-insert
// scenario: a hub_send from a Claude Code session (Brian/Rain/coder) goes
// through the MCP server, which holds its own *DB instance pointing at
// the same SQLite file. The TUI process's db.onMessages callback list
// (where Emma's OnHubMessage is wired in cmd/bot-hq/main.go) does NOT
// fire for those cross-process inserts.
//
// Pre-hotfix (slice 2 merged binary): Emma's only path to see such
// inserts was the boot-time replayThroughSentinel window of the last 50
// messages. Live traffic from MCP-routed hub_sends went undetected.
//
// Post-hotfix: Emma's sentinelPollLoop tick polls the DB directly,
// catching cross-process inserts on the watermark-incremental scan.
//
// The test deliberately does NOT register Emma's OnHubMessage on the DB,
// modeling the cross-process boundary. The success criterion is that
// pollSentinel alone produces a ledger entry.
func TestPollSentinelCatchesCrossProcessInserts(t *testing.T) {
	home := t.TempDir()
	t.Setenv("BOT_HQ_HOME", home)

	dbPath := filepath.Join(t.TempDir(), "test.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()

	g := New(db, hub.GemmaConfig{})
	// Deliberately NOT calling db.OnMessage(g.OnHubMessage) — models the
	// cross-process scenario where MCP-server inserts don't fire the
	// TUI-process callback list.

	if _, err := db.InsertMessage(protocol.Message{
		FromAgent: "brian",
		Type:      protocol.MsgUpdate,
		Content:   "[queue] failed after 5 attempts",
	}); err != nil {
		t.Fatal(err)
	}

	ledgerPath := filepath.Join(home, "sentinels", "queuefail-dryrun.log")
	if data, err := os.ReadFile(ledgerPath); err == nil && len(data) > 0 {
		t.Fatalf("ledger should be empty pre-poll, got %d bytes: %s", len(data), data)
	}

	g.pollSentinel()

	data, err := os.ReadFile(ledgerPath)
	if err != nil {
		t.Fatalf("ledger missing post-poll — cross-process polling not wired: %v", err)
	}
	if !strings.Contains(string(data), "[queue] failed after 5 attempts") {
		t.Errorf("ledger missing expected entry, got: %s", data)
	}
	if !strings.Contains(string(data), "from brian") {
		t.Errorf("ledger missing sender attribution, got: %s", data)
	}
}

// TestPollSentinelWatermarkPreventsReprocess locks the watermark
// invariant: a second poll over the same DB state must NOT re-append
// already-processed messages. Without watermark advancement, polling
// every 5s would flood the ledger with duplicates.
func TestPollSentinelWatermarkPreventsReprocess(t *testing.T) {
	home := t.TempDir()
	t.Setenv("BOT_HQ_HOME", home)

	dbPath := filepath.Join(t.TempDir(), "test.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()

	g := New(db, hub.GemmaConfig{})

	if _, err := db.InsertMessage(protocol.Message{
		FromAgent: "brian",
		Content:   "[queue] failed after 2 attempts",
	}); err != nil {
		t.Fatal(err)
	}

	g.pollSentinel()
	g.pollSentinel()
	g.pollSentinel()

	ledgerPath := filepath.Join(home, "sentinels", "queuefail-dryrun.log")
	data, err := os.ReadFile(ledgerPath)
	if err != nil {
		t.Fatalf("ledger missing: %v", err)
	}
	lines := strings.Split(strings.TrimRight(string(data), "\n"), "\n")
	if len(lines) != 1 {
		t.Errorf("expected exactly 1 ledger line after 3 polls of 1 message; got %d: %q", len(lines), data)
	}
}

// TestAppendToDryRunLedgerDedupsByMsgID locks the cross-bounce dedup
// invariant: when a ledger line embeds a `msg #<id>` token, a subsequent
// append with the same msg-id is a no-op. This handles the "Emma bounces
// and replayThroughSentinel re-fires for the same 50-msg window already
// ledgered in a prior boot" failure mode.
//
// Lines that don't embed `msg #<id>` (free-form helper test calls) are
// not deduped — preserves backwards compatibility with legacy callers.
func TestAppendToDryRunLedgerDedupsByMsgID(t *testing.T) {
	home := t.TempDir()
	t.Setenv("BOT_HQ_HOME", home)

	AppendToDryRunLedger("queuefail", "msg #42 from brian | pattern X | excerpt-A")
	AppendToDryRunLedger("queuefail", "msg #42 from brian | pattern X | excerpt-B")
	AppendToDryRunLedger("queuefail", "msg #43 from rain | pattern X | excerpt-C")

	ledgerPath := filepath.Join(home, "sentinels", "queuefail-dryrun.log")
	data, err := os.ReadFile(ledgerPath)
	if err != nil {
		t.Fatal(err)
	}
	lines := strings.Split(strings.TrimRight(string(data), "\n"), "\n")
	if len(lines) != 2 {
		t.Errorf("expected 2 ledger lines (msg #42 and #43; second #42 deduped), got %d: %q", len(lines), data)
	}

	if !strings.Contains(string(data), "msg #42") {
		t.Errorf("ledger missing msg #42, got: %s", data)
	}
	if !strings.Contains(string(data), "msg #43") {
		t.Errorf("ledger missing msg #43, got: %s", data)
	}
	if !strings.Contains(string(data), "excerpt-A") {
		t.Errorf("first msg #42 entry (excerpt-A) should be preserved, got: %s", data)
	}
	if strings.Contains(string(data), "excerpt-B") {
		t.Errorf("second msg #42 entry (excerpt-B) should be deduped out, got: %s", data)
	}
}

// TestAppendToDryRunLedgerKeepsLegacyCallers locks that lines without
// embedded msg-id tokens are still appended unconditionally. The existing
// TestSentinelDryRunWritesToLedger uses such legacy-shaped lines.
func TestAppendToDryRunLedgerKeepsLegacyCallers(t *testing.T) {
	home := t.TempDir()
	t.Setenv("BOT_HQ_HOME", home)

	AppendToDryRunLedger("queuefail", "free-form observation 1")
	AppendToDryRunLedger("queuefail", "free-form observation 2")

	ledgerPath := filepath.Join(home, "sentinels", "queuefail-dryrun.log")
	data, err := os.ReadFile(ledgerPath)
	if err != nil {
		t.Fatal(err)
	}
	lines := strings.Split(strings.TrimRight(string(data), "\n"), "\n")
	if len(lines) != 2 {
		t.Errorf("legacy free-form lines must not be deduped; expected 2 lines, got %d: %q", len(lines), data)
	}
}

// TestReplayThroughSentinelSetsWatermark locks that boot-replay advances
// the polling watermark past the replayed window. Without this, the
// subsequent first poll would re-process the same 50 messages again.
func TestReplayThroughSentinelSetsWatermark(t *testing.T) {
	home := t.TempDir()
	t.Setenv("BOT_HQ_HOME", home)

	dbPath := filepath.Join(t.TempDir(), "test.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()

	g := New(db, hub.GemmaConfig{})

	var lastID int64
	for i := 0; i < 5; i++ {
		id, err := db.InsertMessage(protocol.Message{
			FromAgent: "brian",
			Content:   fmt.Sprintf("routine message %d", i),
		})
		if err != nil {
			t.Fatal(err)
		}
		lastID = id
	}

	g.replayThroughSentinel()

	if got := g.lastSentinelMsgID; got < lastID {
		t.Errorf("watermark must advance past last replayed msg id; got %d, want >= %d", got, lastID)
	}

	// Subsequent poll over the same DB state should be a no-op — no new
	// messages above watermark, no ledger writes.
	g.pollSentinel()

	ledgerPath := filepath.Join(home, "sentinels", "queuefail-dryrun.log")
	if data, err := os.ReadFile(ledgerPath); err == nil && len(data) > 0 {
		t.Errorf("ledger must be empty when no matching messages; watermark dedup leaking, got: %s", data)
	}
}

// TestPollSentinelHonorsStopCh locks teardown safety: the polling
// goroutine must exit when stopCh is closed (via Stop()). Without this,
// a Gemma instance that panics on Start() leaves a leaked goroutine.
//
// Source-level structural check (mirrors TestStartWiresHeartbeatLoop
// pattern) — full goroutine lifecycle would need an Ollama dep.
func TestPollSentinelHonorsStopCh(t *testing.T) {
	data, err := os.ReadFile("gemma.go")
	if err != nil {
		t.Fatal(err)
	}
	src := string(data)
	if !strings.Contains(src, "go g.sentinelPollLoop()") {
		t.Errorf("gemma.go Start() must contain `go g.sentinelPollLoop()` — sentinel polling not wired")
	}
	if !strings.Contains(src, "func (g *Gemma) sentinelPollLoop()") {
		t.Errorf("gemma.go must define sentinelPollLoop on *Gemma")
	}
	if !strings.Contains(src, "<-g.stopCh") {
		t.Errorf("sentinelPollLoop body must select on stopCh for clean shutdown")
	}
}

// TestSentinelPollIntervalSane locks the cadence floor — must be at
// least 1s to avoid hammering the DB, and at most 30s to keep detection
// latency bounded for the H-22 use case (queue-fails should surface
// within ~one cadence window).
func TestSentinelPollIntervalSane(t *testing.T) {
	if sentinelPollInterval < time.Second {
		t.Errorf("sentinelPollInterval must be >= 1s to prevent DB hammering, got %v", sentinelPollInterval)
	}
	if sentinelPollInterval > 30*time.Second {
		t.Errorf("sentinelPollInterval must be <= 30s to keep H-22 detection latency bounded, got %v", sentinelPollInterval)
	}
}
