package investigate

import (
	"errors"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/faulttree"
	"github.com/gregoryerrl/bot-hq/internal/mvt"
)

// ====== LookupPattern + AllPatterns ======

func TestLookupPattern_knownIDs(t *testing.T) {
	cases := []PatternID{
		PatternBugRecurrence,
		PatternPerfDegradation,
		PatternDriftDetection,
		PatternIntegrationFailure,
		PatternConfigMismatch,
	}
	for _, id := range cases {
		t.Run(string(id), func(t *testing.T) {
			p, err := LookupPattern(id)
			if err != nil {
				t.Fatalf("LookupPattern(%q): %v", id, err)
			}
			if p.ID != id {
				t.Errorf("ID = %q, want %q", p.ID, id)
			}
			if p.Title == "" || p.RootHypothesis == "" {
				t.Errorf("pattern fields incomplete: %+v", p)
			}
			if len(p.ChildConditions) == 0 {
				t.Errorf("pattern %q has no child conditions", id)
			}
		})
	}
}

func TestLookupPattern_unknownReturnsError(t *testing.T) {
	_, err := LookupPattern("imaginary-pattern")
	if err == nil {
		t.Fatal("expected error for unknown pattern")
	}
	if !errors.Is(err, ErrUnknownPattern) {
		t.Errorf("err = %v, want errors.Is ErrUnknownPattern", err)
	}
}

func TestAllPatterns_returnsSortedByID(t *testing.T) {
	all := AllPatterns()
	if len(all) != 5 {
		t.Errorf("len(AllPatterns()) = %d, want 5", len(all))
	}
	for i := 1; i < len(all); i++ {
		if all[i-1].ID >= all[i].ID {
			t.Errorf("AllPatterns not sorted by ID: %q before %q", all[i-1].ID, all[i].ID)
		}
	}
}

// ====== FindHypothesisHint ======

func TestFindHypothesisHint_matchingNodeTitle(t *testing.T) {
	p, _ := LookupPattern(PatternBugRecurrence)
	hint, ok := p.FindHypothesisHint("Prior-fix scope was narrower than the class")
	if !ok {
		t.Fatal("expected hint for matching node title")
	}
	if hint.SuggestedHypothesis == "" {
		t.Errorf("hint.SuggestedHypothesis empty: %+v", hint)
	}
}

func TestFindHypothesisHint_unmatched(t *testing.T) {
	p, _ := LookupPattern(PatternBugRecurrence)
	if _, ok := p.FindHypothesisHint("not a real node title"); ok {
		t.Error("expected no hint for unmatched node title")
	}
}

// ====== InstantiatePattern ======

func TestInstantiatePattern_seedsRootAndChildren(t *testing.T) {
	o := newTestOrchestrator(t)
	inv, err := o.InstantiatePattern(PatternBugRecurrence, PatternParams{
		AgentA:        "brian",
		AgentB:        "rain",
		ModelA:        "claude-default",
		ModelB:        "deepseek-v4-pro",
		DecisionClass: mvt.DecisionMedium,
		Subject:       "msg-12345 cite-drift recurrence",
	})
	if err != nil {
		t.Fatalf("InstantiatePattern: %v", err)
	}
	tree, _ := inv.GetTree()
	pat, _ := LookupPattern(PatternBugRecurrence)
	want := 1 + len(pat.ChildConditions) // root + N children
	if len(tree.Nodes) != want {
		t.Errorf("tree node count = %d, want %d (root+children)", len(tree.Nodes), want)
	}

	// Root should contain Subject parenthetical
	root := tree.GetNode(tree.Nodes[0].ID)
	if root.Owner != "brian" {
		t.Errorf("root owner = %q, want brian", root.Owner)
	}
	if root.Title != pat.RootHypothesis+" (msg-12345 cite-drift recurrence)" {
		t.Errorf("root title = %q", root.Title)
	}
}

func TestInstantiatePattern_subjectOptional(t *testing.T) {
	o := newTestOrchestrator(t)
	inv, err := o.InstantiatePattern(PatternDriftDetection, PatternParams{
		AgentA:        "brian",
		AgentB:        "rain",
		ModelA:        "x",
		ModelB:        "y",
		DecisionClass: mvt.DecisionLow,
	})
	if err != nil {
		t.Fatalf("InstantiatePattern no subject: %v", err)
	}
	tree, _ := inv.GetTree()
	root := tree.Nodes[0]
	pat, _ := LookupPattern(PatternDriftDetection)
	if root.Title != pat.RootHypothesis {
		t.Errorf("root title with empty subject = %q, want unmodified RootHypothesis", root.Title)
	}
}

func TestInstantiatePattern_initialEvidenceSeeded(t *testing.T) {
	o := newTestOrchestrator(t)
	inv, _ := o.InstantiatePattern(PatternDriftDetection, PatternParams{
		AgentA:        "brian",
		AgentB:        "rain",
		ModelA:        "x",
		ModelB:        "y",
		DecisionClass: mvt.DecisionMedium,
		InitialEvidence: []string{
			"msg-17266",
			"file:internal/foo/bar.go:42",
		},
	})
	tree, _ := inv.GetTree()
	root := tree.Nodes[0]
	if len(root.CiteAnchors) != 2 {
		t.Errorf("root CiteAnchors count = %d, want 2", len(root.CiteAnchors))
	}
}

func TestInstantiatePattern_unknownPatternRejected(t *testing.T) {
	o := newTestOrchestrator(t)
	_, err := o.InstantiatePattern("imaginary-zzz", PatternParams{
		AgentA: "brian", AgentB: "rain",
	})
	if err == nil {
		t.Fatal("expected error for unknown pattern")
	}
	if !errors.Is(err, ErrUnknownPattern) {
		t.Errorf("err = %v, want errors.Is ErrUnknownPattern", err)
	}
}

func TestInstantiatePattern_emptyAgentsRejected(t *testing.T) {
	o := newTestOrchestrator(t)
	if _, err := o.InstantiatePattern(PatternBugRecurrence, PatternParams{AgentA: "", AgentB: "rain"}); err == nil {
		t.Error("expected error for empty agentA")
	}
	if _, err := o.InstantiatePattern(PatternBugRecurrence, PatternParams{AgentA: "brian", AgentB: ""}); err == nil {
		t.Error("expected error for empty agentB")
	}
}

func TestInstantiatePattern_sameAgentsRejected(t *testing.T) {
	o := newTestOrchestrator(t)
	if _, err := o.InstantiatePattern(PatternBugRecurrence, PatternParams{
		AgentA: "brian", AgentB: "brian",
		DecisionClass: mvt.DecisionMedium,
	}); err == nil {
		t.Error("expected error for agentA==agentB (R44 anti-cross)")
	}
}

func TestInstantiatePattern_followUpAntiCrossWorks(t *testing.T) {
	// Smoke-test that an instantiated investigation can proceed through
	// AssignAntiCross + StartHypothesisLoop on the seeded children.
	o := newTestOrchestrator(t)
	inv, _ := o.InstantiatePattern(PatternConfigMismatch, PatternParams{
		AgentA:        "brian",
		AgentB:        "rain",
		ModelA:        "x",
		ModelB:        "y",
		DecisionClass: mvt.DecisionMedium,
	})
	tree, _ := inv.GetTree()

	// Pick first child node (not root) for anti-cross assignment
	var childID string
	for _, n := range tree.Nodes {
		if n.ParentID != "" {
			childID = n.ID
			break
		}
	}
	if childID == "" {
		t.Fatal("no child node found in instantiated pattern")
	}

	investigator, err := inv.AssignAntiCross(childID)
	if err != nil {
		t.Fatalf("AssignAntiCross on seeded child: %v", err)
	}
	if investigator != "rain" {
		t.Errorf("investigator = %q, want rain (anti-cross of brian-owner)", investigator)
	}

	// Hypothesis loop should be startable on the assigned node
	loop, err := inv.StartHypothesisLoop(childID, "test-hypothesis-from-pattern-instantiation")
	if err != nil {
		t.Fatalf("StartHypothesisLoop: %v", err)
	}
	if loop.Driver != "rain" {
		t.Errorf("loop driver = %q, want rain", loop.Driver)
	}
}

// ====== Pattern data-shape integrity ======

func TestPatternRegistry_allPatternsHaveValidStructure(t *testing.T) {
	for _, p := range AllPatterns() {
		t.Run(string(p.ID), func(t *testing.T) {
			if p.Title == "" {
				t.Errorf("pattern %q missing Title", p.ID)
			}
			if p.RootHypothesis == "" {
				t.Errorf("pattern %q missing RootHypothesis", p.ID)
			}
			if len(p.ChildConditions) < 2 {
				t.Errorf("pattern %q has <2 child conditions (%d)", p.ID, len(p.ChildConditions))
			}
			for i, c := range p.ChildConditions {
				if c.Title == "" {
					t.Errorf("pattern %q child[%d] missing Title", p.ID, i)
				}
				if c.NodeType != faulttree.NodeAction && c.NodeType != faulttree.NodeCondition {
					t.Errorf("pattern %q child[%d] invalid NodeType %q", p.ID, i, c.NodeType)
				}
			}
		})
	}
}
