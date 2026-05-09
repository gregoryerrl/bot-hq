// Package verify implements T-4 Verify primitives + Rain-Verify-mode-
// expansion per phase-t.md v5.
//
// Two Verify modes:
//
//   - PlanVerify: Rain adversarial review of Plan-doc per T-3 plan.RunPlanVerify
//     (delegated to internal/plan; this package adds prompt-template + audit-trail)
//   - ImplementVerify: Rain adversarial review of Implement output (commits,
//     test results, observability) with sandbox-execute + security-check +
//     downstream-impact analysis
//
// Per phase-t.md v5 R45 EXTENDED: Verify-mode prompt-templates are
// model-aware (Brian-Claude vs Rain-DeepSeek may need different prompt
// framing per per-mode-per-model template registry). Rain has block-
// authority at both checkpoints; Verify-fail loop-back to prior phase via
// IPIV state machine.
//
// Sandbox tech: Testcontainers-Go primary; Playwright with tracing for
// browser-class. Sandbox warm-pool + Verify-result cache (T-4 R53
// efficiency dimension).
//
// MVP scope: prompt-template registry + audit-trail schema + sandbox
// invocation interface (sandbox-impl deferred to dedicated package +
// production deployment).

package verify

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"time"

	"gopkg.in/yaml.v3"
)

// Mode discriminates Plan-Verify vs Implement-Verify.
type Mode string

const (
	ModePlanVerify      Mode = "plan-verify"
	ModeImplementVerify Mode = "implement-verify"
)

// Result captures the Verify verdict.
type Result string

const (
	ResultPending  Result = ""
	ResultPass     Result = "pass"
	ResultFail     Result = "fail"
	ResultEscalate Result = "escalate"
)

// Report is the structured audit-trail of one Verify-cycle.
type Report struct {
	TaskID     string      `yaml:"task_id"`
	Mode       Mode        `yaml:"mode"`
	Verifier   string      `yaml:"verifier"`               // agent-id (typically "rain")
	Model      string      `yaml:"model,omitempty"`         // backend model (e.g. "deepseek-v4-pro")
	Result     Result      `yaml:"result"`
	Reason     string      `yaml:"reason,omitempty"`
	Findings   []Finding   `yaml:"findings,omitempty"`
	SandboxLog string      `yaml:"sandbox_log,omitempty"`   // path to sandbox-execution log
	StartedAt  time.Time   `yaml:"started_at"`
	EndedAt    time.Time   `yaml:"ended_at,omitempty"`
}

// Finding is one issue surfaced by Verify (e.g. test-fail, security-flag,
// downstream-break).
type Finding struct {
	Severity   string `yaml:"severity"` // "info" | "warn" | "block"
	Category   string `yaml:"category"` // "test-failure" | "security" | "regression" | "scope-drift"
	Title      string `yaml:"title"`
	Detail     string `yaml:"detail,omitempty"`
	CiteAnchor string `yaml:"cite_anchor,omitempty"`
}

// NewReport initializes a Verify-cycle audit-trail.
func NewReport(taskID, verifier string, mode Mode, model string) (*Report, error) {
	if taskID == "" || verifier == "" {
		return nil, errors.New("taskID and verifier are required")
	}
	if mode != ModePlanVerify && mode != ModeImplementVerify {
		return nil, fmt.Errorf("invalid mode: %s", mode)
	}
	return &Report{
		TaskID:    taskID,
		Mode:      mode,
		Verifier:  verifier,
		Model:     model,
		Result:    ResultPending,
		StartedAt: time.Now().UTC(),
	}, nil
}

// AddFinding appends a finding.
func (r *Report) AddFinding(severity, category, title, detail, citeAnchor string) {
	r.Findings = append(r.Findings, Finding{
		Severity:   severity,
		Category:   category,
		Title:      title,
		Detail:     detail,
		CiteAnchor: citeAnchor,
	})
}

// Conclude sets the Verify result + reason. EndedAt is auto-set.
func (r *Report) Conclude(result Result, reason string) error {
	switch result {
	case ResultPass, ResultFail, ResultEscalate:
		// ok
	default:
		return fmt.Errorf("invalid result: %s", result)
	}
	r.Result = result
	r.Reason = reason
	r.EndedAt = time.Now().UTC()
	return nil
}

// HasBlockingFindings reports whether any finding has severity=block.
// Block-class findings auto-escalate Verify result to Fail per Verify
// block-authority semantics.
func (r *Report) HasBlockingFindings() bool {
	for _, f := range r.Findings {
		if f.Severity == "block" {
			return true
		}
	}
	return false
}

// Save persists the report.
func (r *Report) Save(path string) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return fmt.Errorf("mkdir: %w", err)
	}
	data, err := yaml.Marshal(r)
	if err != nil {
		return fmt.Errorf("marshal: %w", err)
	}
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, data, 0o644); err != nil {
		return fmt.Errorf("write tmp: %w", err)
	}
	return os.Rename(tmp, path)
}

// Load reads + decodes a Verify report.
func Load(path string) (*Report, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read: %w", err)
	}
	var r Report
	if err := yaml.Unmarshal(data, &r); err != nil {
		return nil, fmt.Errorf("unmarshal: %w", err)
	}
	return &r, nil
}

// CanonicalPath returns the canonical Verify report path for a task.
// Mode is included in filename to disambiguate plan-verify vs implement-
// verify cycles.
func CanonicalPath(homeDir, project, taskID string, mode Mode, sequence int) string {
	name := fmt.Sprintf("%s-%d.yaml", string(mode), sequence)
	return filepath.Join(homeDir, ".bot-hq", "projects", project, "tasks", taskID, "verify-reports", name)
}

// ====== Prompt-template registry (R45 EXTENDED) ======

// PromptTemplate is one Verify-mode prompt-template, optionally model-tuned.
type PromptTemplate struct {
	Mode    Mode
	Model   string // empty for default; set for model-specific override
	Content string
}

var (
	defaultPromptTemplates = []PromptTemplate{
		{
			Mode:  ModePlanVerify,
			Model: "",
			Content: `You are Rain, the bot-hq adversarial QA verifier in Plan-Verify-mode.
Review the attached Plan-doc + investigation-doc + fault-tree.

Apply this checklist (per phase-t.md v5 + plan.RunPlanVerify):
  1. Plan title + summary present?
  2. At least one step?
  3. All step IDs unique?
  4. DependsOn references valid?
  5. Cite-anchors reference investigation-doc OR fault-tree?
  6. Bilateral-mode: are GenuineForks resolved?
  7. Are scope-lock-clauses internally consistent?
  8. Are R-rule discipline references accurate (cite-from-actual)?
  9. Does Plan address all open fault-tree leaves?

Output ONLY JSON: {"verdict": "pass" | "fail" | "escalate", "reason": "...", "findings": [{"severity": "info|warn|block", "category": "...", "title": "...", "detail": "...", "cite_anchor": "..."}, ...]}

You have BLOCK-AUTHORITY at this checkpoint. Plan→Implement transition is gated on your verdict.`,
		},
		{
			Mode:  ModeImplementVerify,
			Model: "",
			Content: `You are Rain, the bot-hq adversarial QA verifier in Implement-Verify-mode.
Review the attached Implement output (commits, test-suite results, observability metrics).

Apply this checklist:
  1. All planned steps implemented?
  2. Tests added + passing?
  3. No regressions in existing test-suite?
  4. Sandbox-execute confirms behavior matches Plan?
  5. No security-class issues introduced?
  6. No downstream-impact-class issues (other components broken)?
  7. Cite-anchors in commit messages valid (cite-from-actual)?
  8. Performance baselines preserved or improved (per R53 efficiency)?

Output ONLY JSON: {"verdict": "pass" | "fail" | "escalate", "reason": "...", "findings": [{...}, ...]}

You have BLOCK-AUTHORITY. Implement→Complete is gated on your verdict.`,
		},
	}
)

// LookupPromptTemplate returns the appropriate prompt-template for the
// (mode, model) tuple. Falls through to default-template when no model-
// specific override exists.
func LookupPromptTemplate(mode Mode, model string) (string, error) {
	// Try exact match first
	for _, t := range defaultPromptTemplates {
		if t.Mode == mode && t.Model == model {
			return t.Content, nil
		}
	}
	// Fall through to default (Model=="")
	for _, t := range defaultPromptTemplates {
		if t.Mode == mode && t.Model == "" {
			return t.Content, nil
		}
	}
	return "", fmt.Errorf("no prompt-template for mode=%s model=%s", mode, model)
}

// AllTemplates returns all registered templates for diagnostic listing.
func AllTemplates() []PromptTemplate {
	out := make([]PromptTemplate, len(defaultPromptTemplates))
	copy(out, defaultPromptTemplates)
	sort.Slice(out, func(i, j int) bool {
		if out[i].Mode != out[j].Mode {
			return out[i].Mode < out[j].Mode
		}
		return out[i].Model < out[j].Model
	})
	return out
}

// ====== Sandbox interface (T-4 minimal abstraction; impl deferred) ======

// Sandbox is the interface T-4 sandbox tech (Testcontainers-Go / Playwright)
// must implement. Verify primitives invoke these to execute tests / repro
// browser interactions in isolation.
type Sandbox interface {
	// Spawn provisions a fresh sandbox + returns a session-handle.
	Spawn() (string, error)
	// Exec runs a command inside the sandbox + returns stdout/stderr.
	Exec(sessionID, cmd string) (stdout string, stderr string, exitCode int, err error)
	// Teardown releases sandbox resources.
	Teardown(sessionID string) error
}

// VerifyResultCache is a minimal in-memory cache for idempotent Verify
// results per phase-t.md v5 R53 efficiency dimension. Production-grade
// persistence + TTL deferred to T-4 implementation expansion.
type VerifyResultCache struct {
	store map[string]*Report
}

// NewVerifyResultCache constructs an empty cache.
func NewVerifyResultCache() *VerifyResultCache {
	return &VerifyResultCache{store: make(map[string]*Report)}
}

// Get retrieves a cached report by deterministic key.
func (c *VerifyResultCache) Get(key string) (*Report, bool) {
	r, ok := c.store[key]
	return r, ok
}

// Put stores a report by key.
func (c *VerifyResultCache) Put(key string, r *Report) {
	c.store[key] = r
}

// Size returns the current cache size.
func (c *VerifyResultCache) Size() int { return len(c.store) }
