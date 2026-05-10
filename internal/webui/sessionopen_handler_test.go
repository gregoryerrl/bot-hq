package webui

import (
	"encoding/json"
	"net/http"
	"path/filepath"
	"strings"
	"testing"
)

// TestHandleSessionOpen_endToEnd asserts the /api/session-open endpoint
// returns a fully-populated payload with overview, rules_resolved, and tasks
// when the supporting files exist on disk. The bootstrap.md surface was
// removed in Phase X-1 (post-Phase-V cleanup).
func TestHandleSessionOpen_endToEnd(t *testing.T) {
	root := t.TempDir()
	mustWrite(t, filepath.Join(root, "README.md"), "# bot-hq\n\nTrio orchestration.\n")

	mustMkdir(t, filepath.Join(root, "rules"))
	mustWrite(t, filepath.Join(root, "rules", "general.yaml"),
		"tone:\n  reply: neutral\ngreenlight:\n  push: explicit verbatim\n")

	mustMkdir(t, filepath.Join(root, "projects"))
	mustWrite(t, filepath.Join(root, "projects", "bot-hq.yaml"),
		"project_name: bot-hq\ngates:\n  push:\n    requiresApproval: false\n")

	mustMkdir(t, filepath.Join(root, "rules", "agents"))
	mustWrite(t, filepath.Join(root, "rules", "agents", "brian.yaml"),
		"role: HANDS\nexec:\n  pushClass: gated\n")

	mustMkdir(t, filepath.Join(root, "projects", "bot-hq"))
	mustWrite(t, filepath.Join(root, "projects", "bot-hq", "tasks.md"),
		"---\ntasks:\n  - id: t1\n    title: Wire session-open\n    status: in_progress\n    owner: brian\n---\n\nTask body.\n")

	s := newTestServer(t, root)
	status, body := callRoute(t, s, "GET", "/api/session-open?project=bot-hq&agent=brian")
	if status != http.StatusOK {
		t.Fatalf("status = %d, want 200; body=%s", status, body)
	}

	var payload map[string]any
	if err := json.Unmarshal([]byte(body), &payload); err != nil {
		t.Fatalf("invalid json: %v\n%s", err, body)
	}

	if payload["project"] != "bot-hq" {
		t.Errorf("project = %v", payload["project"])
	}
	if payload["agent"] != "brian" {
		t.Errorf("agent = %v", payload["agent"])
	}

	overview, _ := payload["overview"].(string)
	if !strings.Contains(overview, "Trio orchestration") {
		t.Errorf("overview missing: %q", overview)
	}

	rules, ok := payload["rules_resolved"].(map[string]any)
	if !ok || rules["agent"] == nil {
		t.Errorf("rules_resolved missing agent layer: %+v", payload["rules_resolved"])
	}
	if rules["greenlight"] == nil {
		t.Errorf("rules_resolved missing general greenlight layer")
	}

	tasks, ok := payload["tasks"].(map[string]any)
	if !ok {
		t.Fatalf("tasks missing")
	}
	tlist, _ := tasks["tasks"].([]any)
	if len(tlist) != 1 {
		t.Errorf("tasks list len = %d", len(tlist))
	}

	stats, _ := payload["stats"].(map[string]any)
	if total, _ := stats["total_tokens"].(float64); total == 0 {
		t.Errorf("stats.total_tokens not populated")
	}
}

// TestHandleSessionOpen_missingFiles asserts the endpoint returns 200 with
// empty sections (not 500) when supporting files are absent.
func TestHandleSessionOpen_missingFiles(t *testing.T) {
	root := t.TempDir()
	mustMkdir(t, filepath.Join(root, "rules"))
	mustWrite(t, filepath.Join(root, "rules", "general.yaml"), "tone:\n  reply: g\n")
	s := newTestServer(t, root)
	status, body := callRoute(t, s, "GET", "/api/session-open?project=bot-hq")
	if status != http.StatusOK {
		t.Fatalf("status = %d, want 200; body=%s", status, body)
	}
	var payload map[string]any
	if err := json.Unmarshal([]byte(body), &payload); err != nil {
		t.Fatal(err)
	}
	if payload["tasks"] != nil {
		t.Errorf("missing tasks should be nil; got %v", payload["tasks"])
	}
}
