// Package daemoncron is the daemon-side scheduling layer for cadence-
// driven hub emits previously owned by the gemma agent.
//
// Phase S S-1a (user msg 15734 + 15760 "all emma's current task ...
// will be deferred to program ... shall not be removed ... programmatically
// done"): replicates 21+ gemma-emit call-sites across 7-8 surface
// categories (heartbeat-ledger / stale-coder / plan-usage 3-sub /
// context-cap-warning / delivery-gap / egress-audit / lifecycle-hooks /
// sentinel-queuefail) using daemon-side cron + DB-state checks. NO
// LLM judgment — pure rule-driven cron/threshold logic.
//
// Architecture: extract-with-delegate per Phase S Rain BRAIN-2nd msg
// 15796. Cron struct owns goroutine lifecycle (Start/Stop + WaitGroup
// drain); each surface registers a goroutine that reads DB state on
// its own cadence + emits via hub.DB.InsertMessage. Existing gemma
// emit-call-sites get feature-flag dual-emit-prevention via
// hub.DB.IsDaemoncronOnline() check (interpretation (ii) per Rain
// msg 15796 PUSH-BACK A — daemoncron owns cadence; gemma emit-call-
// sites short-circuit when daemoncron-online).
//
// Staged migration per Phase S sub-commit plan (Rain msg 15799):
// S-1a-1 (this commit) — skeleton + heartbeat-ledger surface; gemma
// goroutine deletion deferred to S-1b OR S-1a-followup.

package daemoncron

import (
	"context"
	"sync"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
)

// surfaceFunc is the tick-callback signature for each cadence-driven
// surface. Receives the Cron parent for shared db + ctx access; runs
// emit-or-skip logic per surface-specific cadence rules.
type surfaceFunc func(*Cron)

// surface bundles a cadence-driven goroutine: a name (for logging)
// and the tick function. Tickers run on the surface's own cadence;
// surfaceFunc decides emit-or-skip via DB-state check.
type surface struct {
	name string
	tick time.Duration
	fn   surfaceFunc
}

// Cron is the daemon-side cadence scheduler. Owns goroutine lifecycle
// for all daemoncron surfaces. db is the hub.DB used for emit; ctx
// gates goroutine cancellation; wg drains on Stop.
type Cron struct {
	db       *hub.DB
	ctx      context.Context
	cancel   context.CancelFunc
	wg       sync.WaitGroup
	mu       sync.Mutex
	running  bool
	surfaces []surface

	// nowFunc allows tests to inject a mock clock for cadence checks
	// without spawning real tickers. Default time.Now.
	nowFunc func() time.Time
}

// New constructs a Cron tied to the given hub.DB. Surfaces are
// registered via the Register method or pre-built constructor
// helpers (e.g., NewWithDefaults wires the standard surface set).
func New(db *hub.DB) *Cron {
	ctx, cancel := context.WithCancel(context.Background())
	return &Cron{
		db:      db,
		ctx:     ctx,
		cancel:  cancel,
		nowFunc: time.Now,
	}
}

// NewWithDefaults constructs a Cron and pre-registers the standard
// surface set. Z-8a dropped stale-coder (user-flagged as noise; no
// model invocation so it was system-class anyway and the prefix-based
// rule-text dispatch never earned its keep). Heartbeat-ledger stays.
func NewWithDefaults(db *hub.DB) *Cron {
	c := New(db)
	c.Register(surface{
		name: "heartbeat-ledger",
		tick: heartbeatTickInterval,
		fn:   runHeartbeatLedgerSurface,
	})
	return c
}

// Register adds a surface to be spawned on Start. Must be called
// before Start (registration after Start is a programming error and
// silently no-ops to keep the lifecycle simple).
func (c *Cron) Register(s surface) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.running {
		return
	}
	c.surfaces = append(c.surfaces, s)
}

// Start spawns one goroutine per registered surface. Each goroutine
// runs its own ticker + calls the surface's tick function on each
// fire. Stop drains all goroutines via ctx-cancel + WaitGroup.
//
// Idempotent: subsequent calls after first Start are no-ops.
func (c *Cron) Start() error {
	c.mu.Lock()
	if c.running {
		c.mu.Unlock()
		return nil
	}
	c.running = true
	surfaces := c.surfaces
	c.mu.Unlock()

	for _, s := range surfaces {
		c.wg.Add(1)
		go c.runSurface(s)
	}
	return nil
}

// Stop cancels the parent context + waits for all surface goroutines
// to drain. Safe to call when not running (no-op).
func (c *Cron) Stop() error {
	c.mu.Lock()
	if !c.running {
		c.mu.Unlock()
		return nil
	}
	c.running = false
	c.mu.Unlock()

	c.cancel()
	c.wg.Wait()
	return nil
}

// runSurface is the per-surface goroutine driver. Ticks at the
// surface's cadence; calls tick fn each fire; exits on ctx-cancel.
func (c *Cron) runSurface(s surface) {
	defer c.wg.Done()
	ticker := time.NewTicker(s.tick)
	defer ticker.Stop()

	for {
		select {
		case <-c.ctx.Done():
			return
		case <-ticker.C:
			s.fn(c)
		}
	}
}

// IsRunning reports whether Start has been called and Stop hasn't
// completed. Used by gemma feature-flag dual-emit-prevention check.
func (c *Cron) IsRunning() bool {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.running
}

// SetNowFunc injects a clock for tests. Must be called before Start;
// post-Start calls silently no-op.
func (c *Cron) SetNowFunc(fn func() time.Time) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.running {
		return
	}
	c.nowFunc = fn
}

// Now returns the current time per the configured clock (real or
// mock). Surface fns use this for cadence checks.
func (c *Cron) Now() time.Time {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.nowFunc()
}

// DB exposes the underlying hub.DB to surface fns.
func (c *Cron) DB() *hub.DB {
	return c.db
}
