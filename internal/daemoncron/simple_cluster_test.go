package daemoncron

import (
	"strings"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

func TestBuildContextCapCriticalContent_Format(t *testing.T) {
	got := BuildContextCapCriticalContent("brian", 96)
	if !strings.HasPrefix(got, "[CRITICAL]") {
		t.Errorf("missing [CRITICAL] prefix; got %q", got)
	}
	if !strings.Contains(got, "agent brian at 96%") {
		t.Errorf("expected halt-reason substring; got %q", got)
	}
	if !strings.Contains(got, "halt + checkpoint via H-15") {
		t.Errorf("expected halt-trigger recognition substring; got %q", got)
	}
}

func TestBuildContextCapHaltReason_Format(t *testing.T) {
	got := BuildContextCapHaltReason("rain", 95)
	if got != "agent rain at 95%, halt + checkpoint via H-15 + idle for fresh session" {
		t.Errorf("halt-reason format mismatch; got %q", got)
	}
}

func TestBuildDeliveryGapContent_Format(t *testing.T) {
	got := BuildDeliveryGapContent(42, "coder-x", 5*time.Minute, 1001, 3)
	if !strings.HasPrefix(got, "[DELIVERY-GAP]") {
		t.Errorf("missing [DELIVERY-GAP] prefix; got %q", got)
	}
	if !strings.Contains(got, "msg 42 to coder-x") {
		t.Errorf("expected msg-id + target; got %q", got)
	}
	if !strings.Contains(got, "queue-id 1001") {
		t.Errorf("expected queue-id; got %q", got)
	}
	if !strings.Contains(got, "3 attempts") {
		t.Errorf("expected attempts count; got %q", got)
	}
}

func TestBuildEgressAuditContent_Format(t *testing.T) {
	got := BuildEgressAuditContent("brian", 5, "running tests")
	if !strings.HasPrefix(got, "[EGRESS-GAP]") {
		t.Errorf("missing [EGRESS-GAP] prefix; got %q", got)
	}
	if !strings.Contains(got, "agent brian pane advanced over 5 ticks") {
		t.Errorf("expected agent + tick-count; got %q", got)
	}
	if !strings.Contains(got, `"running tests"`) {
		t.Errorf("expected quoted snippet; got %q", got)
	}
}

func TestEmitContextCapCritical_FiresMsgFlagToUser(t *testing.T) {
	db := setupTestDB(t)
	EmitContextCapCritical(db, time.Now(), "brian", 96)
	msgs, _ := db.GetRecentMessages(10)
	for _, m := range msgs {
		if m.FromAgent == simpleClusterAgentID && strings.HasPrefix(m.Content, "[CRITICAL]") {
			if m.Type != protocol.MsgFlag {
				t.Errorf("context-cap critical should be MsgFlag; got %s", m.Type)
			}
			if m.ToAgent != "user" {
				t.Errorf("context-cap critical should target user; got %q", m.ToAgent)
			}
			return
		}
	}
	t.Error("expected context-cap critical emit not found")
}

func TestEmitDeliveryGap_FiresBroadcast(t *testing.T) {
	db := setupTestDB(t)
	EmitDeliveryGap(db, 42, "coder-x", 5*time.Minute, 1001, 3)
	msgs, _ := db.GetRecentMessages(10)
	for _, m := range msgs {
		if m.FromAgent == simpleClusterAgentID && strings.HasPrefix(m.Content, "[DELIVERY-GAP]") {
			if m.Type != protocol.MsgUpdate {
				t.Errorf("delivery-gap should be MsgUpdate; got %s", m.Type)
			}
			if m.ToAgent != "" {
				t.Errorf("delivery-gap should broadcast (empty ToAgent); got %q", m.ToAgent)
			}
			return
		}
	}
	t.Error("expected delivery-gap emit not found")
}

func TestEmitEgressAudit_FiresBroadcast(t *testing.T) {
	db := setupTestDB(t)
	EmitEgressAudit(db, "brian", 5, "running tests")
	msgs, _ := db.GetRecentMessages(10)
	for _, m := range msgs {
		if m.FromAgent == simpleClusterAgentID && strings.HasPrefix(m.Content, "[EGRESS-GAP]") {
			if m.Type != protocol.MsgUpdate {
				t.Errorf("egress-audit should be MsgUpdate; got %s", m.Type)
			}
			return
		}
	}
	t.Error("expected egress-audit emit not found")
}
