// Package plan implements T-3 Plan primitives per phase-t.md v5:
// Plan-solo (Brian-Claude) + Plan-bilateral (Brian-Claude + Rain-DeepSeek
// per R44 expanded) + Plan-Verify-mode + Plan-bilateral merge-algorithm.
//
// Plan consumes investigation-doc + fault-tree (T-2 cl_fault_tree output)
// and produces an architectural design doc that integrates with the
// phase-doc convention. When stakes-class warrants per R44 expanded /
// R47 revised, Plan fires bilaterally: Brian solo-drafts Plan-A; Rain
// solo-drafts Plan-B; merge via shared-checklist + cite-cross-verify;
// converged Plan-doc proceeds to Implement via Rain Plan-Verify-mode
// checkpoint.

package plan

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"gopkg.in/yaml.v3"
)

// Mode discriminates Plan-solo vs Plan-bilateral execution.
type Mode string

const (
	ModeSolo      Mode = "solo"
	ModeBilateral Mode = "bilateral"
)

// VerifyVerdict captures the Rain Plan-Verify-mode block-authority decision.
type VerifyVerdict string

const (
	VerifyPending  VerifyVerdict = ""
	VerifyApproved VerifyVerdict = "approved"
	VerifyRejected VerifyVerdict = "rejected"
)

// Plan is a per-task architectural design doc produced by Plan-class work.
type Plan struct {
	TaskID      string         `yaml:"task_id"`
	Author      string         `yaml:"author"`              // agent-id (Brian for solo; "bilateral-merged" for bilateral)
	Mode        Mode           `yaml:"mode"`
	Title       string         `yaml:"title"`
	Summary     string         `yaml:"summary"`
	Steps       []PlanStep     `yaml:"steps"`
	CiteAnchors []string       `yaml:"cite_anchors,omitempty"` // investigation-doc + fault-tree references
	VerifyVerdict VerifyVerdict `yaml:"verify_verdict"`
	VerifyReason  string        `yaml:"verify_reason,omitempty"`
	CreatedAt   time.Time      `yaml:"created_at"`
	UpdatedAt   time.Time      `yaml:"updated_at"`
}

// PlanStep is one ordered step in the Plan.
type PlanStep struct {
	ID          int    `yaml:"id"`
	Title       string `yaml:"title"`
	Description string `yaml:"description,omitempty"`
	DependsOn   []int  `yaml:"depends_on,omitempty"` // step IDs that must complete first
}

// NewSoloPlan creates a new Plan in solo mode (Brian-Claude default).
func NewSoloPlan(taskID, author, title, summary string) (*Plan, error) {
	if taskID == "" || author == "" || title == "" {
		return nil, errors.New("taskID, author, title are required")
	}
	now := time.Now().UTC()
	return &Plan{
		TaskID:    taskID,
		Author:    author,
		Mode:      ModeSolo,
		Title:     title,
		Summary:   summary,
		Steps:     []PlanStep{},
		CreatedAt: now,
		UpdatedAt: now,
	}, nil
}

// AddStep appends a step + auto-assigns the next sequential ID.
func (p *Plan) AddStep(title, description string, dependsOn []int) PlanStep {
	maxID := 0
	for _, s := range p.Steps {
		if s.ID > maxID {
			maxID = s.ID
		}
	}
	step := PlanStep{
		ID:          maxID + 1,
		Title:       title,
		Description: description,
		DependsOn:   dependsOn,
	}
	p.Steps = append(p.Steps, step)
	p.UpdatedAt = time.Now().UTC()
	return step
}

// AddCiteAnchor appends an evidence cite-anchor (e.g. investigation-doc path,
// fault-tree node-id, msg-id reference).
func (p *Plan) AddCiteAnchor(anchor string) {
	p.CiteAnchors = append(p.CiteAnchors, anchor)
	p.UpdatedAt = time.Now().UTC()
}

// Save persists the Plan to YAML at the canonical path.
func (p *Plan) Save(path string) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return fmt.Errorf("mkdir: %w", err)
	}
	data, err := yaml.Marshal(p)
	if err != nil {
		return fmt.Errorf("marshal: %w", err)
	}
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, data, 0o644); err != nil {
		return fmt.Errorf("write tmp: %w", err)
	}
	return os.Rename(tmp, path)
}

// Load reads + decodes a Plan from YAML.
func Load(path string) (*Plan, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read: %w", err)
	}
	var p Plan
	if err := yaml.Unmarshal(data, &p); err != nil {
		return nil, fmt.Errorf("unmarshal: %w", err)
	}
	return &p, nil
}

// CanonicalPath returns the per-task canonical Plan path.
// Variant=="" stores at plan.yaml; variant="a"/"b" stores at plan-a.yaml/plan-b.yaml
// for bilateral merge inputs.
func CanonicalPath(homeDir, project, taskID, variant string) string {
	name := "plan.yaml"
	if variant != "" {
		name = "plan-" + variant + ".yaml"
	}
	return filepath.Join(homeDir, ".bot-hq", "projects", project, "tasks", taskID, name)
}

// ====== Plan-bilateral merge-algorithm ======

// DivergenceClass classifies how Plan-A and Plan-B differ at a step level.
type DivergenceClass string

const (
	DivAlignment       DivergenceClass = "alignment"        // identical or trivially equivalent
	DivConvergenceNeeded DivergenceClass = "convergence-needed" // similar; can merge programmatically
	DivGenuineFork     DivergenceClass = "genuine-fork"     // material difference; needs user-coordination
)

// MergeReport summarizes the result of Plan-bilateral merge.
type MergeReport struct {
	TaskID         string
	PlanA          *Plan
	PlanB          *Plan
	Merged         *Plan
	Divergences    []Divergence
	GenuineForks   int
	GeneratedAt    time.Time
}

// Divergence is one merge-decision: where Plan-A and Plan-B differ + how
// the merger resolved it (or escalated to user).
type Divergence struct {
	StepIDA  int
	StepIDB  int
	Class    DivergenceClass
	Note     string
	Resolved bool
}

// MergeBilateral runs the Plan-bilateral merge-algorithm per phase-t.md
// v5 R44 expanded. Inputs: Plan-A (Brian-Claude variant) + Plan-B
// (Rain-DeepSeek variant). Output: merged Plan + MergeReport.
//
// Algorithm (per phase-t.md v5):
//  1. Pair steps by title-equivalence (case-insensitive substring overlap)
//  2. Classify each pair: alignment / convergence-needed / genuine-fork
//  3. Auto-merge alignment + convergence-needed pairs
//  4. Surface genuine-fork pairs in MergeReport.Divergences (resolved=false)
//  5. Compose merged Plan with author=bilateral-merged
//
// Caller MUST inspect MergeReport.GenuineForks > 0 → escalate to user
// per R32 SCOPE-FORK-CONFIRMATION before proceeding to Implement.
func MergeBilateral(planA, planB *Plan) (*MergeReport, error) {
	if planA == nil || planB == nil {
		return nil, errors.New("both Plan-A and Plan-B required")
	}
	if planA.TaskID != planB.TaskID {
		return nil, fmt.Errorf("TaskID mismatch: %q vs %q", planA.TaskID, planB.TaskID)
	}

	report := &MergeReport{
		TaskID:      planA.TaskID,
		PlanA:       planA,
		PlanB:       planB,
		GeneratedAt: time.Now().UTC(),
	}

	merged, err := NewSoloPlan(planA.TaskID, "bilateral-merged", planA.Title, planA.Summary)
	if err != nil {
		return nil, fmt.Errorf("init merged: %w", err)
	}
	merged.Mode = ModeBilateral

	// Pair steps by title-substring match
	matchedB := make(map[int]bool)
	for _, sa := range planA.Steps {
		matched := false
		for _, sb := range planB.Steps {
			if matchedB[sb.ID] {
				continue
			}
			if titlesEquivalent(sa.Title, sb.Title) {
				class := classifyDivergence(sa, sb)
				div := Divergence{
					StepIDA: sa.ID,
					StepIDB: sb.ID,
					Class:   class,
				}
				switch class {
				case DivAlignment:
					div.Note = "identical or trivially equivalent"
					div.Resolved = true
					merged.AddStep(sa.Title, sa.Description, sa.DependsOn)
				case DivConvergenceNeeded:
					div.Note = "similar; merged via Plan-A canonical text"
					div.Resolved = true
					merged.AddStep(sa.Title, mergeDescriptions(sa.Description, sb.Description), sa.DependsOn)
				case DivGenuineFork:
					div.Note = "material difference; ESCALATE to user via R32 SCOPE-FORK-CONFIRMATION"
					div.Resolved = false
					report.GenuineForks++
					// Include both variants in merged output for user inspection
					merged.AddStep(sa.Title+" [A-variant]", sa.Description, sa.DependsOn)
					merged.AddStep(sb.Title+" [B-variant]", sb.Description, sb.DependsOn)
				}
				report.Divergences = append(report.Divergences, div)
				matchedB[sb.ID] = true
				matched = true
				break
			}
		}
		if !matched {
			merged.AddStep(sa.Title+" [A-only]", sa.Description, sa.DependsOn)
			report.Divergences = append(report.Divergences, Divergence{
				StepIDA: sa.ID, Class: DivGenuineFork,
				Note: "step exists in Plan-A but not Plan-B; preserved as A-only",
				Resolved: false,
			})
			report.GenuineForks++
		}
	}
	// Pick up any Plan-B steps not yet matched
	for _, sb := range planB.Steps {
		if matchedB[sb.ID] {
			continue
		}
		merged.AddStep(sb.Title+" [B-only]", sb.Description, sb.DependsOn)
		report.Divergences = append(report.Divergences, Divergence{
			StepIDB: sb.ID, Class: DivGenuineFork,
			Note: "step exists in Plan-B but not Plan-A; preserved as B-only",
			Resolved: false,
		})
		report.GenuineForks++
	}

	// Combine cite-anchors (deduped + sorted)
	anchorSet := make(map[string]bool)
	for _, a := range planA.CiteAnchors {
		anchorSet[a] = true
	}
	for _, a := range planB.CiteAnchors {
		anchorSet[a] = true
	}
	var anchors []string
	for a := range anchorSet {
		anchors = append(anchors, a)
	}
	sort.Strings(anchors)
	merged.CiteAnchors = anchors

	report.Merged = merged
	return report, nil
}

// titlesEquivalent reports whether two step titles refer to the same intent
// (case-insensitive trim + min-length-3 substring match either direction).
func titlesEquivalent(a, b string) bool {
	la := strings.ToLower(strings.TrimSpace(a))
	lb := strings.ToLower(strings.TrimSpace(b))
	if la == lb {
		return true
	}
	if len(la) < 3 || len(lb) < 3 {
		return false
	}
	return strings.Contains(la, lb) || strings.Contains(lb, la)
}

// classifyDivergence inspects two paired steps + returns the divergence class.
// Heuristic: identical descriptions = alignment; substring-overlap on
// description = convergence-needed; otherwise genuine-fork.
func classifyDivergence(a, b PlanStep) DivergenceClass {
	if a.Description == b.Description && a.Title == b.Title {
		return DivAlignment
	}
	da := strings.ToLower(strings.TrimSpace(a.Description))
	db := strings.ToLower(strings.TrimSpace(b.Description))
	if da == "" && db == "" {
		return DivAlignment
	}
	if da == "" || db == "" {
		return DivConvergenceNeeded
	}
	if strings.Contains(da, db) || strings.Contains(db, da) {
		return DivConvergenceNeeded
	}
	return DivGenuineFork
}

// mergeDescriptions concatenates two descriptions without duplicating content.
func mergeDescriptions(a, b string) string {
	a = strings.TrimSpace(a)
	b = strings.TrimSpace(b)
	if a == "" {
		return b
	}
	if b == "" {
		return a
	}
	if strings.Contains(a, b) {
		return a
	}
	if strings.Contains(b, a) {
		return b
	}
	return a + "\n\n[B-variant]: " + b
}

// ====== Plan-Verify-mode ======

// VerifyChecklist is the structured Plan-Verify-mode checklist Rain runs
// in adversarial review of a Plan-doc. Each item is a check + result.
type VerifyChecklist struct {
	Items []VerifyItem
}

// VerifyItem is one Plan-Verify check.
type VerifyItem struct {
	Check  string
	Pass   bool
	Reason string
}

// RunPlanVerify executes the standard Plan-Verify-mode checklist on a Plan.
// Returns VerifyVerdict (approved / rejected) + populated checklist for
// audit-trail. Rain has block-authority at this checkpoint per phase-t.md
// v5 (Plan→Implement gated on plan-verify pass).
//
// Standard checks:
//   - Plan has at least one step
//   - All step IDs are unique
//   - DependsOn references are valid
//   - Plan has at least one cite-anchor (investigation-doc OR fault-tree)
//   - Title + Summary are non-empty
//   - When bilateral: GenuineForks == 0 (escalated divergences must be resolved)
func RunPlanVerify(p *Plan, mergeReport *MergeReport) (VerifyVerdict, *VerifyChecklist, string) {
	checklist := &VerifyChecklist{}
	add := func(check string, pass bool, reason string) {
		checklist.Items = append(checklist.Items, VerifyItem{Check: check, Pass: pass, Reason: reason})
	}

	if p == nil {
		add("plan-non-nil", false, "plan is nil")
		return VerifyRejected, checklist, "plan is nil"
	}

	if p.Title == "" {
		add("title-non-empty", false, "title is empty")
	} else {
		add("title-non-empty", true, "")
	}

	if p.Summary == "" {
		add("summary-non-empty", false, "summary is empty")
	} else {
		add("summary-non-empty", true, "")
	}

	if len(p.Steps) < 1 {
		add("at-least-one-step", false, "plan has no steps")
	} else {
		add("at-least-one-step", true, fmt.Sprintf("%d steps", len(p.Steps)))
	}

	idSeen := map[int]bool{}
	dupIDs := false
	for _, s := range p.Steps {
		if idSeen[s.ID] {
			dupIDs = true
			break
		}
		idSeen[s.ID] = true
	}
	add("step-ids-unique", !dupIDs, "")

	depsValid := true
	for _, s := range p.Steps {
		for _, dep := range s.DependsOn {
			if !idSeen[dep] {
				depsValid = false
				break
			}
		}
		if !depsValid {
			break
		}
	}
	add("depends-on-valid", depsValid, "")

	if len(p.CiteAnchors) == 0 {
		add("at-least-one-cite-anchor", false, "no cite-anchors (need investigation-doc or fault-tree reference)")
	} else {
		add("at-least-one-cite-anchor", true, fmt.Sprintf("%d anchors", len(p.CiteAnchors)))
	}

	if p.Mode == ModeBilateral && mergeReport != nil {
		if mergeReport.GenuineForks > 0 {
			add("bilateral-no-genuine-forks", false, fmt.Sprintf("%d genuine forks unresolved (R32 escalation needed)", mergeReport.GenuineForks))
		} else {
			add("bilateral-no-genuine-forks", true, "")
		}
	}

	// Aggregate verdict
	allPass := true
	var failReasons []string
	for _, item := range checklist.Items {
		if !item.Pass {
			allPass = false
			failReasons = append(failReasons, item.Check+": "+item.Reason)
		}
	}
	if allPass {
		p.VerifyVerdict = VerifyApproved
		p.VerifyReason = "all checks pass"
		return VerifyApproved, checklist, "all checks pass"
	}
	reason := "Plan-Verify REJECTED: " + strings.Join(failReasons, "; ")
	p.VerifyVerdict = VerifyRejected
	p.VerifyReason = reason
	return VerifyRejected, checklist, reason
}
