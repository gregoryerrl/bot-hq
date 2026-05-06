package emma

import (
	"strings"
	"testing"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/agents/bootstrap"
	"github.com/gregoryerrl/bot-hq/internal/agents/sessionopen"
	"github.com/gregoryerrl/bot-hq/internal/agents/tasks"
)

func TestFormatSessionOpen_compressesAggressively(t *testing.T) {
	p := &sessionopen.Payload{
		Project:  "bot-hq",
		Agent:    "emma",
		Overview: "# bot-hq\n\nA trio orchestration system for managing parallel agents working on multiple projects.\n",
		Bootstrap: &sessionopen.BootstrapView{
			Frontmatter: bootstrap.Frontmatter{
				LastSessionID:    "bs1",
				PhaseOrMilestone: "phase-n",
				KeyState:         "Brian on bot-hq main; v3.x-2 in flight",
				WriteTrigger:     "graceful",
			},
			Body: "Some long body that should be dropped for emma.",
		},
		RulesResolved: map[string]any{
			"agent": map[string]any{
				"role": "heartbeat-ledger emitter + plan-usage poller",
				"exec": map[string]any{
					"hubWrites":  "PERMITTED for HEARTBEAT-LEDGER cadence",
					"fileWrites": "FORBIDDEN",
				},
			},
		},
		Tasks: &sessionopen.TasksView{
			Tasks: []tasks.Task{{ID: "t1", Title: "x", Status: "pending"}},
			Body:  "task body should not appear",
		},
	}
	out := FormatSessionOpen(p)
	// Should NOT contain bootstrap body, task body, or full rules dump.
	if strings.Contains(out, "long body that should be dropped") {
		t.Errorf("emma should drop bootstrap body")
	}
	if strings.Contains(out, "task body should not appear") {
		t.Errorf("emma should drop tasks body")
	}
	// Should contain compressed essentials.
	if !strings.Contains(out, "Role: heartbeat-ledger") {
		t.Errorf("role missing: %q", out)
	}
	if !strings.Contains(out, "exec.hubWrites:") {
		t.Errorf("exec.hubWrites missing: %q", out)
	}
	if !strings.Contains(out, "phase=phase-n") {
		t.Errorf("bootstrap phase missing: %q", out)
	}
	if !strings.Contains(out, "Active tasks: 1") {
		t.Errorf("task count missing: %q", out)
	}
}

func TestFormatSessionOpen_targetSize(t *testing.T) {
	p := &sessionopen.Payload{
		Project:  "bot-hq",
		Agent:    "emma",
		Overview: strings.Repeat("Lorem ipsum dolor sit amet, consectetur adipiscing elit. ", 200),
		Bootstrap: &sessionopen.BootstrapView{
			Frontmatter: bootstrap.Frontmatter{
				LastSessionID:      "bs1",
				LastSessionCloseAt: time.Now(),
				KeyState:           strings.Repeat("x", 500),
			},
		},
	}
	out := FormatSessionOpen(p)
	approxTokens := len(out) / 4
	if approxTokens > EmmaTokenTarget*2 {
		t.Errorf("emma format too large: ~%d tokens (target %d)", approxTokens, EmmaTokenTarget)
	}
}
