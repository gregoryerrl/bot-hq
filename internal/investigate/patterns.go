// Package investigate — patterns.go: T-8.8 Investigation-pattern-library.
//
// Reusable starter templates for common investigation classes. Builds on
// the T-8.3 Orchestrator + Investigation primitives + faulttree +
// hypothesis primitives. Each Pattern describes a fault-tree shape +
// recommended hypothesis-loop hints; InstantiatePattern opens an
// Investigation + seeds the fault-tree from the template.
//
// Source discriminator (per Rain msg 17268 pre-fire ask): patterns are
// reusable Go data structures built ON TOP of internal/investigate
// foundation. NOT extracted from discipline-log archaeology (Phase V
// scope) and NOT external markdown catalog (T-8.8b followup if needed).

package investigate

import (
	"errors"
	"fmt"
	"sort"

	"github.com/gregoryerrl/bot-hq/internal/faulttree"
	"github.com/gregoryerrl/bot-hq/internal/mvt"
)

// PatternID is a typed string identifying a Pattern. Typed-string preserves
// extensibility (callers can register custom patterns) while giving Go's
// compiler some type-safety vs raw string.
type PatternID string

const (
	PatternBugRecurrence      PatternID = "bug-recurrence"
	PatternPerfDegradation    PatternID = "performance-degradation"
	PatternDriftDetection     PatternID = "drift-detection"
	PatternIntegrationFailure PatternID = "integration-failure"
	PatternConfigMismatch     PatternID = "configuration-mismatch"
)

// ChildCondition describes one fault-tree child node in a Pattern.
type ChildCondition struct {
	Title       string
	Description string
	NodeType    faulttree.NodeType
}

// HypothesisLoopHint suggests a Zeller hypothesis-loop H/P starting point
// for an investigator working on a node matching NodeTitlePattern.
type HypothesisLoopHint struct {
	NodeTitlePattern    string // exact-match against ChildCondition.Title
	SuggestedHypothesis string
	SuggestedPrediction string
}

// Pattern is a reusable investigation template. Each pattern defines a
// root-hypothesis fault-tree shape + child conditions + hypothesis-loop
// hints. Callers instantiate via InstantiatePattern.
type Pattern struct {
	ID                  PatternID
	Title               string
	Description         string
	RootHypothesis      string
	ChildConditions     []ChildCondition
	HypothesisLoopHints []HypothesisLoopHint
}

// PatternParams supplies caller-controlled context at instantiation:
// which two agents drive the investigation + free-form Subject describing
// what's being investigated + optional initial evidence anchors.
type PatternParams struct {
	AgentA          string
	AgentB          string
	ModelA          string
	ModelB          string
	DecisionClass   mvt.DecisionClass
	Subject         string
	InitialEvidence []string
}

// patternRegistry holds the canonical patterns. Lookup via Pattern() helper.
var patternRegistry = map[PatternID]Pattern{
	PatternBugRecurrence: {
		ID:             PatternBugRecurrence,
		Title:          "Bug recurrence",
		Description:    "Recurring failure with prior fix that did not terminate the class. Investigates whether the fix addressed the surface symptom vs the root mechanism.",
		RootHypothesis: "Prior fix addressed surface symptom but did not terminate the recurrence class",
		ChildConditions: []ChildCondition{
			{Title: "Prior-fix scope was narrower than the class", NodeType: faulttree.NodeAction},
			{Title: "Class definition was incomplete or drifted", NodeType: faulttree.NodeCondition},
			{Title: "New code path bypasses prior fix", NodeType: faulttree.NodeAction},
			{Title: "Fix-test coverage missed recurrence dimension", NodeType: faulttree.NodeCondition},
		},
		HypothesisLoopHints: []HypothesisLoopHint{
			{NodeTitlePattern: "Prior-fix scope was narrower than the class", SuggestedHypothesis: "Fix targeted instance N but class includes instances M, N, O", SuggestedPrediction: "Reverting fix and applying class-level fix terminates recurrence"},
		},
	},
	PatternPerfDegradation: {
		ID:             PatternPerfDegradation,
		Title:          "Performance degradation",
		Description:    "Latency or throughput regression. Investigates query-plan / data-volume / IO / lock-contention class.",
		RootHypothesis: "Performance cliff is at <component>:<operation>",
		ChildConditions: []ChildCondition{
			{Title: "Query plan changed (index loss or stats stale)", NodeType: faulttree.NodeAction},
			{Title: "Data volume crossed performance-cliff threshold", NodeType: faulttree.NodeCondition},
			{Title: "Lock-contention from concurrent writers", NodeType: faulttree.NodeAction},
			{Title: "External dependency latency increased", NodeType: faulttree.NodeCondition},
		},
		HypothesisLoopHints: []HypothesisLoopHint{
			{NodeTitlePattern: "Query plan changed (index loss or stats stale)", SuggestedHypothesis: "EXPLAIN shows seq-scan replaced index-scan", SuggestedPrediction: "ANALYZE + index-rebuild restores index-scan plan"},
		},
	},
	PatternDriftDetection: {
		ID:             PatternDriftDetection,
		Title:          "Drift detection (R31/R37 class)",
		Description:    "Cite-claim or estimate drift. Investigates whether drift is mechanical (toolgate gap) or behavioral (discipline-application failure).",
		RootHypothesis: "Drift root-cause is <mechanical-gap-or-behavioral-pattern>",
		ChildConditions: []ChildCondition{
			{Title: "Source-of-truth verification was skipped at emit-time", NodeType: faulttree.NodeAction},
			{Title: "Toolgate gap allowed drift through PreToolUse hook", NodeType: faulttree.NodeCondition},
			{Title: "Peer-cross-check did not catch drift before emit", NodeType: faulttree.NodeAction},
			{Title: "Recurrence pattern across N+ instances", NodeType: faulttree.NodeCondition},
		},
		HypothesisLoopHints: []HypothesisLoopHint{
			{NodeTitlePattern: "Source-of-truth verification was skipped at emit-time", SuggestedHypothesis: "Drafter cited from session-recall not cite-from-actual", SuggestedPrediction: "Mechanical pre-emit hub_read or grep verification breaks recurrence"},
		},
	},
	PatternIntegrationFailure: {
		ID:             PatternIntegrationFailure,
		Title:          "Integration failure",
		Description:    "Cross-system bug — failure at boundary between components or services.",
		RootHypothesis: "Failure occurs at <componentA>↔<componentB> boundary contract mismatch",
		ChildConditions: []ChildCondition{
			{Title: "Schema or contract mismatch between sender and receiver", NodeType: faulttree.NodeAction},
			{Title: "Auth or credential propagation failure across boundary", NodeType: faulttree.NodeCondition},
			{Title: "Rate-limit or quota exceeded silently dropping requests", NodeType: faulttree.NodeAction},
			{Title: "Network or DNS issue intermittently failing handshake", NodeType: faulttree.NodeCondition},
		},
		HypothesisLoopHints: []HypothesisLoopHint{
			{NodeTitlePattern: "Schema or contract mismatch between sender and receiver", SuggestedHypothesis: "Sender emits field X; receiver expects field X' (renamed)", SuggestedPrediction: "Schema-diff between deployed sender + receiver shows field-rename"},
		},
	},
	PatternConfigMismatch: {
		ID:             PatternConfigMismatch,
		Title:          "Configuration mismatch",
		Description:    "Environment / config drift causing observed-vs-expected divergence. Investigates env-var / config-file / build-flag class.",
		RootHypothesis: "Observed-vs-expected divergence root-causes to <env|config|build> mismatch",
		ChildConditions: []ChildCondition{
			{Title: "Environment variable not propagated to subprocess", NodeType: faulttree.NodeAction},
			{Title: "Config file precedence overrode expected value", NodeType: faulttree.NodeCondition},
			{Title: "Build-flag or feature-flag toggled unexpectedly", NodeType: faulttree.NodeAction},
			{Title: "Stale cache served pre-config-change values", NodeType: faulttree.NodeCondition},
		},
		HypothesisLoopHints: []HypothesisLoopHint{
			{NodeTitlePattern: "Environment variable not propagated to subprocess", SuggestedHypothesis: "Parent shell has var; subprocess Env field excludes it", SuggestedPrediction: "exec.Command env-leak fix propagates parent env"},
		},
	},
}

// ErrUnknownPattern is returned when the requested PatternID is not registered.
var ErrUnknownPattern = errors.New("unknown pattern")

// LookupPattern fetches a Pattern by ID. Returns ErrUnknownPattern when
// the ID is not registered.
func LookupPattern(id PatternID) (Pattern, error) {
	p, ok := patternRegistry[id]
	if !ok {
		return Pattern{}, fmt.Errorf("%w: %q", ErrUnknownPattern, id)
	}
	return p, nil
}

// AllPatterns returns the canonical pattern catalog sorted by ID for
// deterministic UI listing + tests.
func AllPatterns() []Pattern {
	out := make([]Pattern, 0, len(patternRegistry))
	for _, p := range patternRegistry {
		out = append(out, p)
	}
	sort.Slice(out, func(i, j int) bool { return out[i].ID < out[j].ID })
	return out
}

// FindHypothesisHint returns the first HypothesisLoopHint matching the
// given node title (exact match against NodeTitlePattern). Returns
// (HypothesisLoopHint{}, false) when no hint matches.
func (p Pattern) FindHypothesisHint(nodeTitle string) (HypothesisLoopHint, bool) {
	for _, h := range p.HypothesisLoopHints {
		if h.NodeTitlePattern == nodeTitle {
			return h, true
		}
	}
	return HypothesisLoopHint{}, false
}

// InstantiatePattern opens a fresh Investigation from the given pattern +
// seeds the fault-tree with the root-hypothesis + all child conditions
// owned by AgentA. Caller subsequently calls AssignAntiCross + StartHypothesisLoop
// + AdvanceHypothesisLoop on each leaf to drive the investigation through
// to convergence.
func (o *Orchestrator) InstantiatePattern(id PatternID, params PatternParams) (*Investigation, error) {
	pat, err := LookupPattern(id)
	if err != nil {
		return nil, err
	}
	if params.AgentA == "" || params.AgentB == "" {
		return nil, errors.New("agentA and agentB are required")
	}
	if params.AgentA == params.AgentB {
		return nil, errors.New("agentA and agentB must differ (R44 anti-cross)")
	}
	inv, err := o.OpenInvestigation(params.DecisionClass, params.AgentA, params.AgentB, params.ModelA, params.ModelB)
	if err != nil {
		return nil, fmt.Errorf("OpenInvestigation: %w", err)
	}
	rootTitle := pat.RootHypothesis
	if params.Subject != "" {
		rootTitle = pat.RootHypothesis + " (" + params.Subject + ")"
	}
	rootID, err := inv.ProposeHypothesis(params.AgentA, rootTitle, pat.Description, "", faulttree.NodeAction)
	if err != nil {
		return nil, fmt.Errorf("seed root: %w", err)
	}
	for _, evid := range params.InitialEvidence {
		if err := inv.AddCiteAnchor(rootID, evid); err != nil {
			return nil, fmt.Errorf("seed evidence: %w", err)
		}
	}
	for _, child := range pat.ChildConditions {
		if _, err := inv.ProposeHypothesis(params.AgentA, child.Title, child.Description, rootID, child.NodeType); err != nil {
			return nil, fmt.Errorf("seed child %q: %w", child.Title, err)
		}
	}
	return inv, nil
}
