package sessionopen

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func setup(t *testing.T) string {
	t.Helper()
	root := t.TempDir()
	must := func(p, c string) {
		if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(p, []byte(c), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	must(filepath.Join(root, "README.md"), "# bot-hq\n\nA duo orchestration system.\n")
	must(filepath.Join(root, "rules", "general.yaml"), "tone:\n  reply: neutral\n")
	must(filepath.Join(root, "projects", "bot-hq.yaml"), "project_name: bot-hq\ngates:\n  push:\n    requiresApproval: false\n")
	must(filepath.Join(root, "rules", "agents", "brian.yaml"), "role: HANDS\nexec:\n  pushClass: gated\n")
	must(filepath.Join(root, "projects", "bot-hq", "tasks.md"), "---\ntasks:\n  - id: t1\n    title: Wire session-open\n    status: in_progress\n    owner: brian\n---\n\nTask notes.\n")
	return root
}

func TestBuild_allFields(t *testing.T) {
	root := setup(t)
	p, err := Build(root, "bot-hq", "brian")
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(p.Overview, "duo orchestration") {
		t.Errorf("overview not loaded: %q", p.Overview)
	}
	if p.Tasks == nil || len(p.Tasks.Tasks) != 1 || p.Tasks.Tasks[0].ID != "t1" {
		t.Errorf("tasks missing: %+v", p.Tasks)
	}
	if p.RulesResolved == nil || p.RulesResolved["agent"] == nil {
		t.Errorf("rules missing or no agent layer: %+v", p.RulesResolved)
	}
	if p.Stats.TotalTokens == 0 {
		t.Errorf("stats not populated")
	}
}

func TestBuild_missingFiles_partialPayload(t *testing.T) {
	root := t.TempDir()
	// Only general.yaml — everything else missing.
	if err := os.MkdirAll(filepath.Join(root, "rules"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, "rules", "general.yaml"), []byte("tone: {reply: g}\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	p, err := Build(root, "bot-hq", "")
	if err != nil {
		t.Fatalf("missing files should not error: %v", err)
	}
	if p.Overview != "" {
		t.Errorf("missing overview should be empty, got %q", p.Overview)
	}
	if p.Tasks != nil {
		t.Errorf("missing tasks should be nil, got %+v", p.Tasks)
	}
}

func TestTruncate_underCap_preserves(t *testing.T) {
	got, tok := truncate("short", 100)
	if got != "short" || tok != approxTokens("short") {
		t.Errorf("under cap should preserve: got %q tok %d", got, tok)
	}
}

func TestTruncate_overCap_trimsAndMarks(t *testing.T) {
	long := strings.Repeat("abcdefgh", 200) // ~400 tokens at 4ch/tok
	got, tok := truncate(long, 50)          // 50 token cap
	if !strings.Contains(got, "[truncated") {
		t.Errorf("over-cap should mark: %q", got[len(got)-100:])
	}
	if tok != 50 {
		t.Errorf("tok should be soft-cap when truncated: got %d", tok)
	}
}

func TestFormatClaude_containsSentinels(t *testing.T) {
	root := setup(t)
	p, err := Build(root, "bot-hq", "brian")
	if err != nil {
		t.Fatal(err)
	}
	out := FormatClaude(p)
	if !strings.Contains(out, "BOT-HQ SESSION-OPEN BEGIN") || !strings.Contains(out, "BOT-HQ SESSION-OPEN END") {
		t.Errorf("missing BEGIN/END sentinels: %q", out[:200])
	}
	if !strings.Contains(out, "## Project overview") {
		t.Errorf("overview header missing")
	}
	if !strings.Contains(out, "## Active tasks") {
		t.Errorf("tasks header missing")
	}
	if !strings.Contains(out, "Wire session-open") {
		t.Errorf("task title missing")
	}
}
