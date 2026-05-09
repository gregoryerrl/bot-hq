package cl

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func newTestCL(t *testing.T) *CL {
	t.Helper()
	root := t.TempDir()
	// Seed minimal CL skeleton
	must := func(p string, content string) {
		full := filepath.Join(root, p)
		if err := os.MkdirAll(filepath.Dir(full), 0o755); err != nil {
			t.Fatalf("mkdir %s: %v", full, err)
		}
		if err := os.WriteFile(full, []byte(content), 0o644); err != nil {
			t.Fatalf("write %s: %v", full, err)
		}
	}
	must("phase/phase-t.md", "# Phase T scope-lock-doc\n")
	must("phase/phase-s.md", "# Phase S scope-lock-doc\n")
	must("ratchets/active.md", "# Active ratchets\n")
	must("gates/pre-commit-checklist.md", "# Pre-commit\n")
	must("discipline-log.md", "# Discipline log\n")
	must("tasks.md", "# Tasks\n")
	must("glossary.md", "# Glossary\n")
	must("roles.md", "# Roles\n")
	must("brian/last_state.json", `{"agent_id":"brian","phase":"Phase T v5"}`)
	must("rain/last_state.json", `{"agent_id":"rain","phase":"Phase T v5"}`)
	must("rules/general.yaml", "version: 1\n")
	must("projects/bot-hq/README.md", "# bot-hq project\n")

	cl, err := NewCL(root)
	if err != nil {
		t.Fatalf("NewCL: %v", err)
	}
	return cl
}

func TestNewCL_defaultsToHomeBotHq(t *testing.T) {
	// Don't actually open ~/.bot-hq in tests; just verify constructor doesn't panic
	// when home dir resolution succeeds AND ~/.bot-hq exists.
	if _, err := os.Stat(filepath.Join(os.Getenv("HOME"), ".bot-hq")); err == nil {
		cl, err := NewCL("")
		if err != nil {
			t.Errorf("NewCL(\"\") with existing ~/.bot-hq should succeed: %v", err)
		}
		if cl != nil && !strings.HasSuffix(cl.Root(), ".bot-hq") {
			t.Errorf("Root() = %q, want suffix .bot-hq", cl.Root())
		}
	}
}

func TestNewCL_invalidRoot_errors(t *testing.T) {
	_, err := NewCL("/nonexistent/path/xyz/.bot-hq")
	if err == nil {
		t.Error("expected error for nonexistent root")
	}
}

func TestNewCL_rootIsFile_errors(t *testing.T) {
	dir := t.TempDir()
	filePath := filepath.Join(dir, "file.txt")
	if err := os.WriteFile(filePath, []byte("hi"), 0o644); err != nil {
		t.Fatalf("write: %v", err)
	}
	_, err := NewCL(filePath)
	if err == nil {
		t.Error("expected error when root is a file (not directory)")
	}
}

func TestPathFor_phase(t *testing.T) {
	cl := newTestCL(t)
	got, err := cl.PathFor(ClassPhase, "phase-t")
	if err != nil {
		t.Fatalf("PathFor: %v", err)
	}
	want := filepath.Join(cl.Root(), "phase", "phase-t.md")
	if got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}

func TestPathFor_ratchetActive(t *testing.T) {
	cl := newTestCL(t)
	got, _ := cl.PathFor(ClassRatchet, "active")
	want := filepath.Join(cl.Root(), "ratchets", "active.md")
	if got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}

func TestPathFor_agentState(t *testing.T) {
	cl := newTestCL(t)
	got, _ := cl.PathFor(ClassAgentState, "brian")
	want := filepath.Join(cl.Root(), "brian", "last_state.json")
	if got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}

func TestPathFor_ipivState(t *testing.T) {
	cl := newTestCL(t)
	got, _ := cl.PathFor(ClassIPIVState, "bot-hq/task-abc")
	want := filepath.Join(cl.Root(), "projects", "bot-hq", "task-abc", "ipiv-state.yaml")
	if got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}

func TestPathFor_unsupportedClass(t *testing.T) {
	cl := newTestCL(t)
	_, err := cl.PathFor(ClassUnknown, "anything")
	if err == nil {
		t.Error("expected ErrUnsupportedClass")
	}
}

func TestGet_existingPhase(t *testing.T) {
	cl := newTestCL(t)
	a, err := cl.Get(ClassPhase, "phase-t")
	if err != nil {
		t.Fatalf("Get: %v", err)
	}
	if a.Class != ClassPhase {
		t.Errorf("class = %q, want phase", a.Class)
	}
	if a.ID != "phase-t" {
		t.Errorf("id = %q, want phase-t", a.ID)
	}
	if !strings.Contains(string(a.Content), "Phase T scope-lock-doc") {
		t.Errorf("content missing expected substring")
	}
	if !a.Loaded {
		t.Error("Loaded should be true after Get")
	}
}

func TestGet_missing_returnsErrNotFound(t *testing.T) {
	cl := newTestCL(t)
	_, err := cl.Get(ClassPhase, "nonexistent")
	if !errors.Is(err, ErrNotFound) {
		t.Errorf("err = %v, want wrap of ErrNotFound", err)
	}
}

func TestList_phaseClass(t *testing.T) {
	cl := newTestCL(t)
	arts, err := cl.List(ClassPhase)
	if err != nil {
		t.Fatalf("List: %v", err)
	}
	if len(arts) != 2 {
		t.Errorf("phase count = %d, want 2 (phase-t + phase-s)", len(arts))
	}
	// Sorted: phase-s before phase-t
	if arts[0].ID != "phase-s" || arts[1].ID != "phase-t" {
		t.Errorf("expected sorted [phase-s, phase-t], got [%s, %s]", arts[0].ID, arts[1].ID)
	}
}

func TestList_referenceDocsClass(t *testing.T) {
	cl := newTestCL(t)
	arts, err := cl.List(ClassReference)
	if err != nil {
		t.Fatalf("List: %v", err)
	}
	if len(arts) < 2 {
		t.Errorf("reference doc count = %d, want >= 2 (glossary + roles)", len(arts))
	}
	seenGlossary := false
	for _, a := range arts {
		if a.ID == "glossary" {
			seenGlossary = true
		}
	}
	if !seenGlossary {
		t.Error("expected glossary in reference list")
	}
}

func TestList_classWithoutDirReturnsEmpty(t *testing.T) {
	cl := newTestCL(t)
	// Gates/* dir exists in seed; remove ratchets/ to test empty-dir behavior
	os.RemoveAll(filepath.Join(cl.Root(), "ratchets"))
	arts, err := cl.List(ClassRatchet)
	if err != nil {
		t.Fatalf("List with missing dir: %v (should be nil error, empty result)", err)
	}
	if len(arts) != 0 {
		t.Errorf("expected empty list, got %d", len(arts))
	}
}

func TestRead_byPath(t *testing.T) {
	cl := newTestCL(t)
	path := filepath.Join(cl.Root(), "phase", "phase-t.md")
	a, err := cl.Read(path)
	if err != nil {
		t.Fatalf("Read: %v", err)
	}
	if a.Class != ClassPhase {
		t.Errorf("class = %q, want phase", a.Class)
	}
	if a.ID != "phase-t" {
		t.Errorf("id = %q, want phase-t", a.ID)
	}
}

func TestWrite_atomicCreate(t *testing.T) {
	cl := newTestCL(t)
	path := filepath.Join(cl.Root(), "phase", "phase-new.md")
	a := &Artifact{
		Class:   ClassPhase,
		ID:      "phase-new",
		Path:    path,
		Content: []byte("# New phase\n"),
	}
	if err := cl.Write(a); err != nil {
		t.Fatalf("Write: %v", err)
	}
	// Verify file exists + has correct content
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("re-read: %v", err)
	}
	if string(data) != "# New phase\n" {
		t.Errorf("content mismatch: %q", string(data))
	}
	// Verify .tmp gone
	if _, err := os.Stat(path + ".tmp"); !os.IsNotExist(err) {
		t.Errorf(".tmp leak: %v", err)
	}
}

func TestWrite_requiresPath(t *testing.T) {
	cl := newTestCL(t)
	err := cl.Write(&Artifact{Content: []byte("x")})
	if err == nil {
		t.Error("expected error when Path is empty")
	}
}

func TestLoadAgentState_brianRoundTrip(t *testing.T) {
	cl := newTestCL(t)
	state, err := cl.LoadAgentState("brian")
	if err != nil {
		t.Fatalf("LoadAgentState: %v", err)
	}
	if state["agent_id"] != "brian" {
		t.Errorf("agent_id = %v, want brian", state["agent_id"])
	}
	if state["phase"] != "Phase T v5" {
		t.Errorf("phase = %v, want 'Phase T v5'", state["phase"])
	}
}

func TestWalk_visitsAllArtifactsAndSkipsRuntimeEphemera(t *testing.T) {
	cl := newTestCL(t)

	// Add some runtime-ephemera that should be skipped
	must := func(p, content string) {
		full := filepath.Join(cl.Root(), p)
		_ = os.MkdirAll(filepath.Dir(full), 0o755)
		_ = os.WriteFile(full, []byte(content), 0o644)
	}
	must("hub.db", "fake-db")
	must("live.log", "log content")
	must("debug.log", "debug content")
	must("bridge/trace.jsonl", "trace")
	must("diag/whatever.json", "diag")

	visited := map[Class]int{}
	err := cl.Walk(func(a *Artifact) error {
		visited[a.Class]++
		return nil
	})
	if err != nil {
		t.Fatalf("Walk: %v", err)
	}

	// Verify expected classes seen
	if visited[ClassPhase] < 2 {
		t.Errorf("phase visited = %d, want >= 2", visited[ClassPhase])
	}
	if visited[ClassRatchet] < 1 {
		t.Errorf("ratchet visited = %d, want >= 1", visited[ClassRatchet])
	}
	if visited[ClassReference] < 2 {
		t.Errorf("reference visited = %d, want >= 2", visited[ClassReference])
	}
	if visited[ClassDisciplineLog] < 1 {
		t.Errorf("discipline-log visited = %d, want 1", visited[ClassDisciplineLog])
	}
	if visited[ClassAgentState] < 2 {
		t.Errorf("agent-state visited = %d, want 2 (brian + rain)", visited[ClassAgentState])
	}
	if visited[ClassUnknown] != 0 {
		t.Errorf("unknown visited = %d, want 0 (Walk should skip unknown)", visited[ClassUnknown])
	}
}

func TestDetectClass_knownPaths(t *testing.T) {
	cl := newTestCL(t)
	cases := []struct {
		path  string
		want  Class
	}{
		{filepath.Join(cl.Root(), "phase/phase-t.md"), ClassPhase},
		{filepath.Join(cl.Root(), "ratchets/active.md"), ClassRatchet},
		{filepath.Join(cl.Root(), "gates/pre-commit-checklist.md"), ClassGate},
		{filepath.Join(cl.Root(), "discipline-log.md"), ClassDisciplineLog},
		{filepath.Join(cl.Root(), "tasks.md"), ClassTasks},
		{filepath.Join(cl.Root(), "glossary.md"), ClassReference},
		{filepath.Join(cl.Root(), "brian/last_state.json"), ClassAgentState},
		{filepath.Join(cl.Root(), "rules/general.yaml"), ClassRule},
		{filepath.Join(cl.Root(), "projects/bot-hq/README.md"), ClassProject},
		{filepath.Join(cl.Root(), "projects/bot-hq/task-abc/ipiv-state.yaml"), ClassIPIVState},
	}
	for _, tc := range cases {
		got := cl.detectClass(tc.path)
		if got != tc.want {
			t.Errorf("detectClass(%q) = %q, want %q", tc.path, got, tc.want)
		}
	}
}

func TestDeriveID_knownPaths(t *testing.T) {
	cl := newTestCL(t)
	cases := []struct {
		path string
		want string
	}{
		{filepath.Join(cl.Root(), "phase/phase-t.md"), "phase-t"},
		{filepath.Join(cl.Root(), "ratchets/active.md"), "active"},
		{filepath.Join(cl.Root(), "rules/general.yaml"), "general"},
		{filepath.Join(cl.Root(), "brian/last_state.json"), "brian"},
		{filepath.Join(cl.Root(), "glossary.md"), "glossary"},
	}
	for _, tc := range cases {
		got := cl.deriveID(tc.path)
		if got != tc.want {
			t.Errorf("deriveID(%q) = %q, want %q", tc.path, got, tc.want)
		}
	}
}
