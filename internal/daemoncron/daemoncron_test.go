package daemoncron

import (
	"path/filepath"
	"sync/atomic"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
)

// setupTestDB creates an isolated hub.DB at t.TempDir per R39
// TEST-ISOLATION. Returns the db handle; t.Cleanup closes.
func setupTestDB(t *testing.T) *hub.DB {
	t.Helper()
	dbPath := filepath.Join(t.TempDir(), "hub.db")
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		t.Fatalf("open hub db: %v", err)
	}
	t.Cleanup(func() { _ = db.Close() })
	return db
}

func TestNew_NotRunningInitially(t *testing.T) {
	db := setupTestDB(t)
	c := New(db)
	if c.IsRunning() {
		t.Error("Cron should not be running pre-Start")
	}
}

func TestStart_TransitionsToRunning(t *testing.T) {
	db := setupTestDB(t)
	c := New(db)
	if err := c.Start(); err != nil {
		t.Fatalf("Start: %v", err)
	}
	if !c.IsRunning() {
		t.Error("Cron should be running after Start")
	}
	if err := c.Stop(); err != nil {
		t.Fatalf("Stop: %v", err)
	}
	if c.IsRunning() {
		t.Error("Cron should not be running after Stop")
	}
}

func TestStart_Idempotent(t *testing.T) {
	db := setupTestDB(t)
	c := New(db)
	_ = c.Start()
	// Second call should be no-op (no error, no panic).
	if err := c.Start(); err != nil {
		t.Errorf("second Start should be idempotent; got %v", err)
	}
	_ = c.Stop()
}

func TestStop_SafeWhenNotRunning(t *testing.T) {
	db := setupTestDB(t)
	c := New(db)
	if err := c.Stop(); err != nil {
		t.Errorf("Stop on not-running Cron should be safe; got %v", err)
	}
}

func TestRegister_AppendsSurface(t *testing.T) {
	db := setupTestDB(t)
	c := New(db)
	c.Register(surface{name: "test", tick: 1 * time.Hour, fn: func(*Cron) {}})
	if len(c.surfaces) != 1 {
		t.Errorf("expected 1 surface registered, got %d", len(c.surfaces))
	}
}

func TestRegister_AfterStartIgnored(t *testing.T) {
	db := setupTestDB(t)
	c := New(db)
	_ = c.Start()
	defer c.Stop()
	before := len(c.surfaces)
	c.Register(surface{name: "late", tick: 1 * time.Hour, fn: func(*Cron) {}})
	if len(c.surfaces) != before {
		t.Errorf("Register after Start should be ignored; surfaces grew %d → %d", before, len(c.surfaces))
	}
}

func TestSurfaceFires(t *testing.T) {
	db := setupTestDB(t)
	c := New(db)
	var fireCount atomic.Int64
	c.Register(surface{
		name: "test-fire",
		tick: 20 * time.Millisecond,
		fn: func(*Cron) {
			fireCount.Add(1)
		},
	})
	_ = c.Start()
	// Wait for at least 2 ticks.
	time.Sleep(70 * time.Millisecond)
	_ = c.Stop()
	got := fireCount.Load()
	if got < 2 {
		t.Errorf("expected surface to fire ≥2 times in 70ms with 20ms tick; got %d", got)
	}
}

func TestSurfaceDrainOnStop(t *testing.T) {
	db := setupTestDB(t)
	c := New(db)
	var fireCount atomic.Int64
	c.Register(surface{
		name: "drain",
		tick: 10 * time.Millisecond,
		fn: func(*Cron) {
			fireCount.Add(1)
		},
	})
	_ = c.Start()
	time.Sleep(40 * time.Millisecond)
	_ = c.Stop()
	preStop := fireCount.Load()
	// Wait for any stale ticks to land.
	time.Sleep(30 * time.Millisecond)
	postStop := fireCount.Load()
	if postStop != preStop {
		t.Errorf("surface fired post-Stop; preStop=%d postStop=%d (expected drain)", preStop, postStop)
	}
}

func TestSetNowFunc_BeforeStart(t *testing.T) {
	db := setupTestDB(t)
	c := New(db)
	mock := time.Date(2026, 5, 8, 12, 0, 0, 0, time.UTC)
	c.SetNowFunc(func() time.Time { return mock })
	got := c.Now()
	if !got.Equal(mock) {
		t.Errorf("Now() should return injected mock; got %v want %v", got, mock)
	}
}

func TestSetNowFunc_AfterStartIgnored(t *testing.T) {
	db := setupTestDB(t)
	c := New(db)
	_ = c.Start()
	defer c.Stop()
	original := c.Now()
	c.SetNowFunc(func() time.Time { return time.Date(1999, 1, 1, 0, 0, 0, 0, time.UTC) })
	got := c.Now()
	if got.Year() == 1999 {
		t.Errorf("SetNowFunc post-Start should be ignored; got year %d", got.Year())
	}
	_ = original
}

func TestNewWithDefaults_RegistersHeartbeat(t *testing.T) {
	db := setupTestDB(t)
	c := NewWithDefaults(db)
	found := false
	for _, s := range c.surfaces {
		if s.name == "heartbeat-ledger" {
			found = true
			break
		}
	}
	if !found {
		t.Error("NewWithDefaults should register heartbeat-ledger surface")
	}
}
