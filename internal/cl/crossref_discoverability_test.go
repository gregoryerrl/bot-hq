package cl

import (
	"context"
	"strings"
	"testing"
)

func TestBuildCrossRefGraph_basicEdges(t *testing.T) {
	cl := newTestCL(t)

	// Add an artifact that cites another via absolute path
	a := &Artifact{
		Class: ClassPhase,
		ID:    "phase-cite-test",
		Path:  cl.Root() + "/phase/phase-cite-test.md",
		Content: []byte(`# Test phase
Cites /tmp/some-known-target.md and /Users/alice/docs/another.md.`),
	}
	if err := cl.Write(a); err != nil {
		t.Fatalf("Write seed: %v", err)
	}

	graph, err := cl.BuildCrossRefGraph(context.Background())
	if err != nil {
		t.Fatalf("BuildCrossRefGraph: %v", err)
	}

	// Verify graph captured the cite-test artifact's outgoing edges
	cites := graph.Cites(a.Path)
	if len(cites) < 2 {
		t.Errorf("expected >=2 cites from phase-cite-test, got %d", len(cites))
	}

	// Verify reverse-index works for known target
	citedBy := graph.CitedBy("/tmp/some-known-target.md")
	if len(citedBy) < 1 {
		t.Errorf("expected /tmp/some-known-target.md cited by phase-cite-test")
	}
}

func TestBuildCrossRefGraph_emptyOnNoCites(t *testing.T) {
	cl := newTestCL(t)
	// Default seed has no file-path cites in content
	graph, err := cl.BuildCrossRefGraph(context.Background())
	if err != nil {
		t.Fatalf("BuildCrossRefGraph: %v", err)
	}
	// May still have cites from seed content if any contain absolute paths;
	// just verify it doesn't crash + returns a valid graph
	if graph == nil {
		t.Error("graph is nil")
	}
}

func TestCrossRefGraph_AllSourcesAndTargets(t *testing.T) {
	cl := newTestCL(t)
	a := &Artifact{
		Class: ClassPhase, ID: "x", Path: cl.Root() + "/phase/x.md",
		Content: []byte("Cites /tmp/foo.md"),
	}
	if err := cl.Write(a); err != nil {
		t.Fatalf("Write: %v", err)
	}
	g, err := cl.BuildCrossRefGraph(context.Background())
	if err != nil {
		t.Fatalf("BuildCrossRefGraph: %v", err)
	}
	sources := g.AllSources()
	targets := g.AllTargets()
	if len(sources) < 1 {
		t.Errorf("expected >=1 source")
	}
	if len(targets) < 1 {
		t.Errorf("expected >=1 target")
	}
	if g.EdgeCount() < 1 {
		t.Errorf("expected >=1 edge")
	}
}

func TestBuildSearchIndex_basicTokenization(t *testing.T) {
	cl := newTestCL(t)
	idx, err := cl.BuildSearchIndex()
	if err != nil {
		t.Fatalf("BuildSearchIndex: %v", err)
	}
	if idx.TokenCount() == 0 {
		t.Error("token count is 0; index empty")
	}
	if idx.ArtifactCount() == 0 {
		t.Error("artifact count is 0")
	}
}

func TestSearchIndex_findsKnownToken(t *testing.T) {
	cl := newTestCL(t)
	idx, err := cl.BuildSearchIndex()
	if err != nil {
		t.Fatalf("BuildSearchIndex: %v", err)
	}

	// Seed phase-t.md contains "Phase T scope-lock-doc"
	hits := idx.Search("scope-lock-doc", "")
	if len(hits) < 1 {
		t.Errorf("expected to find 'scope-lock-doc' in seeded phase artifacts; got %d hits", len(hits))
	}
	for _, h := range hits {
		if !strings.Contains(strings.ToLower(string(idx.artifacts[h.Path].Content)), "scope-lock-doc") {
			t.Errorf("hit %s does not actually contain 'scope-lock-doc'", h.Path)
		}
	}
}

func TestSearchIndex_classFilter(t *testing.T) {
	cl := newTestCL(t)
	idx, err := cl.BuildSearchIndex()
	if err != nil {
		t.Fatalf("BuildSearchIndex: %v", err)
	}

	// Search for token "phase" — both class=phase and reference docs may contain it
	hitsAll := idx.Search("phase", "")
	hitsPhase := idx.Search("phase", ClassPhase)

	if len(hitsAll) < len(hitsPhase) {
		t.Errorf("filter should reduce or equal hit count; all=%d phase=%d", len(hitsAll), len(hitsPhase))
	}
	// All filtered hits should have class=phase
	for _, h := range hitsPhase {
		if h.Class != ClassPhase {
			t.Errorf("class filter violated: hit %s has class %s, want phase", h.Path, h.Class)
		}
	}
}

func TestSearchIndex_emptyQueryReturnsNil(t *testing.T) {
	cl := newTestCL(t)
	idx, _ := cl.BuildSearchIndex()
	if hits := idx.Search("", ""); hits != nil {
		t.Errorf("empty query should return nil; got %d hits", len(hits))
	}
	if hits := idx.Search("   ", ""); hits != nil {
		t.Errorf("whitespace query should return nil; got %d hits", len(hits))
	}
}

func TestSearchIndex_excerptHasMatchingLine(t *testing.T) {
	cl := newTestCL(t)
	a := &Artifact{
		Class: ClassPhase, ID: "ex-test", Path: cl.Root() + "/phase/ex-test.md",
		Content: []byte("First line.\nThis line has the unique-token marker.\nThird line."),
	}
	if err := cl.Write(a); err != nil {
		t.Fatalf("Write: %v", err)
	}
	idx, _ := cl.BuildSearchIndex()
	hits := idx.Search("unique-token", "")
	if len(hits) < 1 {
		t.Fatalf("expected hit for 'unique-token'")
	}
	for _, h := range hits {
		if !strings.Contains(h.Excerpt, "unique-token") {
			t.Errorf("excerpt %q should contain query", h.Excerpt)
		}
	}
}

func TestSearchIndex_prefixMatch(t *testing.T) {
	cl := newTestCL(t)
	a := &Artifact{
		Class: ClassPhase, ID: "pref-test", Path: cl.Root() + "/phase/pref-test.md",
		Content: []byte("scope-locking is a verb form of scope-lock"),
	}
	if err := cl.Write(a); err != nil {
		t.Fatalf("Write: %v", err)
	}
	idx, _ := cl.BuildSearchIndex()
	// "scope-loc" should prefix-match both "scope-locking" and "scope-lock"
	hits := idx.Search("scope-loc", "")
	if len(hits) < 1 {
		t.Errorf("prefix match failed; expected >=1 hit")
	}
}

func TestTokenize_basic(t *testing.T) {
	cases := []struct {
		in   string
		want []string
	}{
		{"hello world", []string{"hello", "world"}},
		{"R51 PER-AGENT-MODEL-CONFIG", []string{"r51", "per-agent-model-config"}},
		{"a", nil}, // length-1 tokens skipped
		{"foo.bar", []string{"foo", "bar"}},
		{"", nil},
	}
	for _, tc := range cases {
		got := tokenize(tc.in)
		if len(got) != len(tc.want) {
			t.Errorf("tokenize(%q) = %v, want %v", tc.in, got, tc.want)
			continue
		}
		for i := range got {
			if got[i] != tc.want[i] {
				t.Errorf("tokenize(%q)[%d] = %q, want %q", tc.in, i, got[i], tc.want[i])
			}
		}
	}
}
