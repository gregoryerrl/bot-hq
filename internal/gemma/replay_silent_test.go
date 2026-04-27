package gemma

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// TestReplaySilentDoesNotArmHysteresis locks the slice-3 #4 invariant:
// boot-replay must NOT arm flagHistory for matched patterns. Pre-fix
// (slice 2 closure cycle msgs 3463/3467), replayThroughSentinel called
// OnHubMessage which armed shouldFlag for the full 30-min hysteresis
// window, blocking subsequent live triggers. Post-fix replay calls
// OnHubMessageReplay which writes ledger but skips shouldFlag.
func TestReplaySilentDoesNotArmHysteresis(t *testing.T) {
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
		Type:      protocol.MsgUpdate,
		Content:   "[queue] failed after 5 attempts",
	}); err != nil {
		t.Fatal(err)
	}

	g.replayThroughSentinel()

	g.flagMu.Lock()
	defer g.flagMu.Unlock()
	if len(g.flagHistory) != 0 {
		t.Errorf("replay armed flagHistory; expected empty, got %d entries: %v", len(g.flagHistory), g.flagHistory)
	}
}

// TestReplaySilentWritesDryRunLedger locks the correctness invariant:
// boot-replay still writes ledger entries for dry-run patterns so
// cross-bounce dedup works. Only the hysteresis-arming side-effect is
// suppressed; the ledger-write side-effect is preserved.
func TestReplaySilentWritesDryRunLedger(t *testing.T) {
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
		Type:      protocol.MsgUpdate,
		Content:   "[queue] failed after 7 attempts",
	}); err != nil {
		t.Fatal(err)
	}

	g.replayThroughSentinel()

	ledgerPath := filepath.Join(home, "sentinels", "queuefail-dryrun.log")
	data, err := os.ReadFile(ledgerPath)
	if err != nil {
		t.Fatalf("ledger missing post-replay — silent-mode dropped ledger write: %v", err)
	}
	if !strings.Contains(string(data), "[queue] failed after 7 attempts") {
		t.Errorf("ledger missing expected entry, got: %s", data)
	}
}

// TestReplayThenLiveDispatchFiresShouldFlag locks the no-false-suppress
// invariant: a live OnHubMessage after replay must fire shouldFlag
// (not blocked by replay since replay never armed flagHistory).
func TestReplayThenLiveDispatchFiresShouldFlag(t *testing.T) {
	home := t.TempDir()
	t.Setenv("BOT_HQ_HOME", home)

	dbPath := filepath.Join(t.TempDir(), "test.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()

	g := New(db, hub.GemmaConfig{})

	// Boot-replay window: matching msg already in DB.
	if _, err := db.InsertMessage(protocol.Message{
		FromAgent: "brian",
		Type:      protocol.MsgUpdate,
		Content:   "[queue] failed after 3 attempts",
	}); err != nil {
		t.Fatal(err)
	}
	g.replayThroughSentinel()

	// Verify replay didn't arm hysteresis.
	g.flagMu.Lock()
	if len(g.flagHistory) != 0 {
		g.flagMu.Unlock()
		t.Fatalf("replay armed flagHistory unexpectedly: %v", g.flagHistory)
	}
	g.flagMu.Unlock()

	// Live trigger post-boot — different message with same pattern.
	live := protocol.Message{
		ID:        9999,
		FromAgent: "rain",
		Type:      protocol.MsgUpdate,
		Content:   "[queue] failed after 4 attempts",
	}
	g.OnHubMessage(live)

	// Live path should have armed flagHistory (shouldFlag fired).
	g.flagMu.Lock()
	defer g.flagMu.Unlock()
	if len(g.flagHistory) == 0 {
		t.Errorf("live OnHubMessage did not arm flagHistory; replay may be blocking live path")
	}
}

// TestReplaySilentSkipsAlwaysFlag locks the no-Discord-spam invariant:
// boot-replay of always-flag patterns must NOT emit MsgFlag (which
// would push to Discord with stale historical content).
func TestReplaySilentSkipsAlwaysFlag(t *testing.T) {
	home := t.TempDir()
	t.Setenv("BOT_HQ_HOME", home)

	dbPath := filepath.Join(t.TempDir(), "test.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()

	g := New(db, hub.GemmaConfig{})

	// Insert an always-flag-shaped message via SentinelMatch's known
	// always-flag patterns. If none of the current sentinel patterns
	// are AlwaysFlag (slice 2 has all observation-only), this test
	// becomes a no-op canary that documents the intent — once an
	// always-flag pattern lands, the test exercises the silent-skip
	// path.
	matched := false
	for _, content := range []string{
		"[queue] failed after 2 attempts",
	} {
		d := SentinelMatch(protocol.Message{Content: content})
		if d.Match && d.AlwaysFlag {
			matched = true
			if _, err := db.InsertMessage(protocol.Message{
				FromAgent: "brian",
				Type:      protocol.MsgUpdate,
				Content:   content,
			}); err != nil {
				t.Fatal(err)
			}
			break
		}
	}

	g.replayThroughSentinel()

	// Count post-replay MsgFlag rows (if any). Should be zero
	// regardless of whether matched: when matched=true, replay must
	// silent-skip; when matched=false (current state), no AlwaysFlag
	// path exercised.
	msgs, err := db.GetRecentMessages(50)
	if err != nil {
		t.Fatal(err)
	}
	for _, m := range msgs {
		if m.Type == protocol.MsgFlag && m.FromAgent == "emma" {
			t.Errorf("replay emitted MsgFlag from emma; expected silent-skip (matched=%v): %+v", matched, m)
		}
	}

	// Hysteresis must remain unarmed regardless.
	g.flagMu.Lock()
	defer g.flagMu.Unlock()
	for k := range g.flagHistory {
		if strings.HasPrefix(k, "sentinel:") {
			t.Errorf("replay armed always-flag hysteresis %q; expected silent-skip", k)
		}
	}
}
