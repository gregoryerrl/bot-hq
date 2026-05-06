// Package sessionopen builds the session-open payload served by
// GET /api/session-open?project=X&agent=Y. The payload bundles the four
// session-init artifacts an agent needs at start:
//
//   - overview:       project overview markdown (README.md or projects/<p>/overview.md)
//   - bootstrap:      last-session bootstrap.md (frontmatter + body)
//   - rules_resolved: deep-merged general → project → agent rules
//   - tasks:          tasks.md frontmatter + body
//
// Per design-spike §2.2: hard cap 5K tokens; soft target 2K; truncation
// from tail with marker. This package is harness-agnostic — adapters
// (claude_adapter / emma) format the payload for their respective
// SessionStart hook surfaces.
package sessionopen

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/gregoryerrl/bot-hq/internal/agents/bootstrap"
	"github.com/gregoryerrl/bot-hq/internal/agents/tasks"
	"github.com/gregoryerrl/bot-hq/internal/rules"
)

// Soft and hard token caps per design-spike §2.2. ~4 chars/token.
const (
	SoftTokenCap = 2000
	HardTokenCap = 5000

	OverviewSoftCap  = 250
	BootstrapSoftCap = 750
	RulesSoftCap     = 1200
	TasksSoftCap     = 600
)

// Payload is the JSON shape returned by /api/session-open.
type Payload struct {
	Project       string         `json:"project"`
	Agent         string         `json:"agent"`
	Overview      string         `json:"overview"`
	Bootstrap     *BootstrapView `json:"bootstrap,omitempty"`
	RulesResolved map[string]any `json:"rules_resolved"`
	Tasks         *TasksView     `json:"tasks,omitempty"`
	Stats         Stats          `json:"stats"`
}

// BootstrapView is a JSON-friendly projection of bootstrap.Bootstrap.
type BootstrapView struct {
	Frontmatter bootstrap.Frontmatter `json:"frontmatter"`
	Body        string                `json:"body"`
}

// TasksView is a JSON-friendly projection of tasks.File.
type TasksView struct {
	Tasks []tasks.Task `json:"tasks"`
	Body  string       `json:"body"`
}

// Stats reports approximate token counts per section + truncation flags.
type Stats struct {
	OverviewTokens  int  `json:"overview_tokens"`
	BootstrapTokens int  `json:"bootstrap_tokens"`
	RulesTokens     int  `json:"rules_tokens"`
	TasksTokens     int  `json:"tasks_tokens"`
	TotalTokens     int  `json:"total_tokens"`
	Truncated       bool `json:"truncated"`
	OverHardCap     bool `json:"over_hard_cap"`
}

// Build assembles the session-open payload for project + agent against
// canonRoot (~/.bot-hq). Truncates each section to its soft cap; flags
// hard-cap-exceed in Stats but does not error (caller decides 400 policy).
func Build(canonRoot, project, agent string) (*Payload, error) {
	if project == "" {
		project = "bot-hq"
	}
	p := &Payload{Project: project, Agent: agent}

	// Overview — bot-hq=README.md; others=projects/<p>/overview.md.
	overview, err := readOverview(canonRoot, project)
	if err != nil {
		return nil, fmt.Errorf("overview: %w", err)
	}
	p.Overview, p.Stats.OverviewTokens = truncate(overview, OverviewSoftCap)

	// Bootstrap.
	if bs, err := bootstrap.Read(canonRoot, project); err != nil {
		return nil, fmt.Errorf("bootstrap: %w", err)
	} else if bs != nil {
		body, btoks := truncate(bs.Body, BootstrapSoftCap)
		p.Bootstrap = &BootstrapView{Frontmatter: bs.Frontmatter, Body: body}
		p.Stats.BootstrapTokens = btoks
	}

	// Rules resolved.
	merged, err := rules.Resolve(canonRoot, project, agent)
	if err != nil {
		return nil, fmt.Errorf("rules: %w", err)
	}
	p.RulesResolved = merged
	p.Stats.RulesTokens = approxTokensOfMap(merged)
	if p.Stats.RulesTokens > RulesSoftCap {
		// Rules are structured; we don't truncate the map (keeps semantic intact)
		// but flag the bloat for the caller.
		p.Stats.Truncated = true
	}

	// Tasks.
	if tf, err := tasks.Read(canonRoot, project); err != nil {
		return nil, fmt.Errorf("tasks: %w", err)
	} else if tf != nil {
		body, ttoks := truncate(tf.Body, TasksSoftCap)
		p.Tasks = &TasksView{Tasks: tf.Frontmatter.Tasks, Body: body}
		p.Stats.TasksTokens = ttoks
	}

	p.Stats.TotalTokens = p.Stats.OverviewTokens + p.Stats.BootstrapTokens + p.Stats.RulesTokens + p.Stats.TasksTokens
	if p.Stats.TotalTokens > HardTokenCap {
		p.Stats.OverHardCap = true
	}

	return p, nil
}

func readOverview(canonRoot, project string) (string, error) {
	var path string
	if project == "bot-hq" {
		path = filepath.Join(canonRoot, "README.md")
	} else {
		path = filepath.Join(canonRoot, "projects", project, "overview.md")
	}
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return "", nil
		}
		return "", err
	}
	return string(data), nil
}

// approxTokens returns tokens at ~4 chars/token (Claude tokenizer rule-of-thumb).
func approxTokens(s string) int {
	if s == "" {
		return 0
	}
	return (len(s) + 3) / 4
}

// approxTokensOfMap rough-estimates tokens for a marshaled YAML/JSON map.
// Pessimistic: counts every key + value byte once.
func approxTokensOfMap(m map[string]any) int {
	var sum int
	for k, v := range m {
		sum += approxTokens(k) + 1
		switch tv := v.(type) {
		case string:
			sum += approxTokens(tv)
		case map[string]any:
			sum += approxTokensOfMap(tv)
		case []any:
			for _, x := range tv {
				if s, ok := x.(string); ok {
					sum += approxTokens(s) + 1
				} else {
					sum += 4
				}
			}
		default:
			sum += 4
		}
	}
	return sum
}

// truncate returns (s', tokens) where s' is at most softCap tokens
// (~4*softCap chars). Truncation snips from tail with a marker.
func truncate(s string, softCap int) (string, int) {
	tok := approxTokens(s)
	if tok <= softCap {
		return s, tok
	}
	maxChars := softCap * 4
	marker := fmt.Sprintf("\n\n[truncated: %d tokens elided]\n", tok-softCap)
	if maxChars > len(marker) {
		maxChars -= len(marker)
	}
	if maxChars < 0 {
		maxChars = 0
	}
	if maxChars > len(s) {
		maxChars = len(s)
	}
	return strings.TrimRight(s[:maxChars], "\n") + marker, softCap
}
