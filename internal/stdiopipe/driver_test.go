package stdiopipe

import (
	"path/filepath"
	"strings"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/hub"
)

func newTestDB(t *testing.T) *hub.DB {
	t.Helper()
	dir := t.TempDir()
	db, err := hub.OpenDB(filepath.Join(dir, "hub.db"))
	if err != nil {
		t.Fatalf("OpenDB: %v", err)
	}
	t.Cleanup(func() { _ = db.Close() })
	return db
}

func TestNew_validation(t *testing.T) {
	db := newTestDB(t)
	if _, err := New(nil, "rain"); err == nil {
		t.Error("expected error for nil db")
	}
	if _, err := New(db, ""); err == nil {
		t.Error("expected error for empty agentID")
	}
}

func TestNew_loadsRainDeepSeekConfig(t *testing.T) {
	db := newTestDB(t)
	d, err := New(db, "rain")
	if err != nil {
		t.Fatalf("New rain: %v", err)
	}
	if d.AgentID() != "rain" {
		t.Errorf("AgentID = %q", d.AgentID())
	}
	cfg := d.Config()
	if cfg.Provider != "deepseek" {
		t.Errorf("provider = %q, want deepseek", cfg.Provider)
	}
	if cfg.ModelName != "deepseek-v4-pro" {
		t.Errorf("model = %q, want deepseek-v4-pro", cfg.ModelName)
	}
}

func TestNew_loadsBrianClaudeConfig(t *testing.T) {
	db := newTestDB(t)
	d, err := New(db, "brian")
	if err != nil {
		t.Fatalf("New brian: %v", err)
	}
	cfg := d.Config()
	if cfg.Provider != "anthropic" {
		t.Errorf("provider = %q, want anthropic", cfg.Provider)
	}
}

func TestNew_unknownAgentFallthroughToClaudeOAuth(t *testing.T) {
	db := newTestDB(t)
	d, err := New(db, "custom-agent-xyz")
	if err != nil {
		t.Fatalf("New custom: %v", err)
	}
	cfg := d.Config()
	if cfg.Provider != "anthropic" {
		t.Errorf("default provider = %q, want anthropic (Claude OAuth fallthrough)", cfg.Provider)
	}
	if cfg.AuthSecretRef != "oauth:CLAUDE_CODE_OAUTH_TOKEN" {
		t.Errorf("default secret-ref = %q", cfg.AuthSecretRef)
	}
}

func TestSendBeforeStart_errors(t *testing.T) {
	db := newTestDB(t)
	d, _ := New(db, "rain")
	if err := d.Send("prompt"); err == nil {
		t.Error("Send before Start should error")
	}
}

func TestReceiveBeforeStart_errors(t *testing.T) {
	db := newTestDB(t)
	d, _ := New(db, "rain")
	if _, err := d.Receive(); err == nil {
		t.Error("Receive before Start should error")
	}
}

func TestWaitBeforeStart_errors(t *testing.T) {
	db := newTestDB(t)
	d, _ := New(db, "rain")
	if _, err := d.Wait(); err == nil {
		t.Error("Wait before Start should error")
	}
}

// ====== PreInjectionContext tests ======

func TestPreInjectionContext_composeIncludesAllSections(t *testing.T) {
	ctx := &PreInjectionContext{
		AgentID:       "brian",
		SessionAnchor: "Phase T v5 in flight; T-6 stdio-pipe driver active",
		RecentMsgIDs:  []int64{17146, 17162, 17196},
		ActivePhase:   "phase-t.md v5",
		CustomBlocks:  []string{"## Custom block\nFOO=BAR"},
	}
	out := ctx.Compose()

	if !strings.Contains(out, "agent=brian") {
		t.Errorf("compose missing agent header")
	}
	if !strings.Contains(out, "phase-t.md v5") {
		t.Errorf("compose missing active phase")
	}
	if !strings.Contains(out, "Phase T v5 in flight") {
		t.Errorf("compose missing session anchor")
	}
	if !strings.Contains(out, "17146") || !strings.Contains(out, "17162") {
		t.Errorf("compose missing peer-coord msg-ids")
	}
	if !strings.Contains(out, "Custom block") {
		t.Errorf("compose missing custom block")
	}
	if !strings.Contains(out, "END PRE-INJECTION CONTEXT") {
		t.Errorf("compose missing closing marker")
	}
}

func TestPreInjectionContext_emptySectionsSkipped(t *testing.T) {
	ctx := &PreInjectionContext{AgentID: "brian"}
	out := ctx.Compose()
	if strings.Contains(out, "Last session anchor") {
		t.Errorf("empty SessionAnchor should not render section")
	}
	if strings.Contains(out, "Recent peer-coord") {
		t.Errorf("empty RecentMsgIDs should not render section")
	}
}
