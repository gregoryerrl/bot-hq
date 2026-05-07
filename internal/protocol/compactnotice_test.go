package protocol

import (
	"strings"
	"testing"
)

func TestMsgCompactNotice_Valid(t *testing.T) {
	if !MsgCompactNotice.Valid() {
		t.Error("MsgCompactNotice should be Valid()")
	}
}

func TestMsgResume_Valid(t *testing.T) {
	if !MsgResume.Valid() {
		t.Error("MsgResume should be Valid()")
	}
}

func TestMsgCompactNotice_IsActive(t *testing.T) {
	if !MsgCompactNotice.IsActive() {
		t.Error("MsgCompactNotice should be in ActiveMessageTypes")
	}
}

func TestMsgResume_IsActive(t *testing.T) {
	if !MsgResume.IsActive() {
		t.Error("MsgResume should be in ActiveMessageTypes")
	}
}

func TestAgentActiveFireInFlight_Empty(t *testing.T) {
	if AgentActiveFireInFlight(Agent{ID: "brian", CurrentTask: ""}) {
		t.Error("empty current_task should report idle (not active fire-in-flight)")
	}
}

func TestAgentActiveFireInFlight_NonEmpty(t *testing.T) {
	if !AgentActiveFireInFlight(Agent{ID: "brian", CurrentTask: "smoking S-2 batch"}) {
		t.Error("non-empty current_task should report active fire-in-flight")
	}
}

func TestAnyPeerActiveFireInFlight_AllIdle(t *testing.T) {
	agents := []Agent{
		{ID: "brian", CurrentTask: ""},
		{ID: "rain", CurrentTask: ""},
		{ID: "emma", CurrentTask: ""},
	}
	if AnyPeerActiveFireInFlight("brian", agents) {
		t.Error("all-idle should report no peer active")
	}
}

func TestAnyPeerActiveFireInFlight_OnePeerActive(t *testing.T) {
	agents := []Agent{
		{ID: "brian", CurrentTask: ""},
		{ID: "rain", CurrentTask: "BRAIN-2nd cite-from-actual"},
		{ID: "emma", CurrentTask: ""},
	}
	if !AnyPeerActiveFireInFlight("brian", agents) {
		t.Error("rain active should report peer active for brian")
	}
}

func TestAnyPeerActiveFireInFlight_SelfActiveOnly(t *testing.T) {
	// Self-active should NOT trigger peer-active flag (compacting agent
	// is the one firing the notice; self's own current_task is
	// expected and not relevant to discriminator).
	agents := []Agent{
		{ID: "brian", CurrentTask: "smoking S-2 batch"},
		{ID: "rain", CurrentTask: ""},
		{ID: "emma", CurrentTask: ""},
	}
	if AnyPeerActiveFireInFlight("brian", agents) {
		t.Error("self-only-active should NOT trigger peer-active discriminator")
	}
}

func TestAnyPeerActiveFireInFlight_EmptyList(t *testing.T) {
	if AnyPeerActiveFireInFlight("brian", nil) {
		t.Error("empty agent list should report no peer active")
	}
}

func TestBuildCompactNoticeContent_PeerActive(t *testing.T) {
	got := BuildCompactNoticeContent("brian", true)
	if !strings.HasPrefix(got, "[HR] ") {
		t.Errorf("peer-active should produce [HR]-prefixed content; got %q", got)
	}
	if !strings.Contains(got, "brian|compacting") {
		t.Errorf("content should include agent-id and compacting marker; got %q", got)
	}
}

func TestBuildCompactNoticeContent_BothIdle(t *testing.T) {
	got := BuildCompactNoticeContent("brian", false)
	if strings.HasPrefix(got, "[HR]") {
		t.Errorf("both-idle should NOT produce [HR]-prefixed content; got %q", got)
	}
	if got != "brian|compacting" {
		t.Errorf("both-idle should be plain compact-pipe format; got %q", got)
	}
}

func TestBuildResumeContent(t *testing.T) {
	got := BuildResumeContent("brian")
	if got != "brian|resumed-context-reloaded" {
		t.Errorf("resume content format mismatch; got %q", got)
	}
	if strings.HasPrefix(got, "[HR]") {
		t.Errorf("resume content should NEVER carry [HR] (always informational); got %q", got)
	}
}
