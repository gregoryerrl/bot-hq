// Package contextload assembles the per-project context blob the trio
// loads when pivoting to work on a project. Replaces the auto-bootstrap
// loop's role of "tell agents what they're working on" with an
// explicit, on-demand mechanism.
//
// Architecture:
//   - Layer 1 (trio OS) lives in agent prompts; Layer 1 is loaded at
//     spawn and applies regardless of project.
//   - Layer 2 (project context) is what this package produces. Read
//     when the user pivots to a project (R20/R16 resume; user msg
//     "let's work on X"; topic-pivot trigger). Single MCP / CLI tool
//     surface. Returned as markdown so the agent internalizes it
//     directly into its working context.
//   - Layer 3 (just-in-time) is everything else read on-demand via
//     Read / hub_read / Skill tool.
//
// Per Phase V architecture (post-CL-refactor): replaces the timer-based
// runBootstrapDefensiveLoop with event-driven CL state + on-demand
// context_load reads. The CL itself is the durable state; this package
// produces a focused projection of it for agent consumption.
package contextload

import (
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/rules"
)

// Context is the assembled per-project view loaded at pivot. Prefer
// Markdown() for agent consumption; the typed fields are exposed for
// tests and for callers that want structured access (e.g., webui).
type Context struct {
	Project  string         // project key (matches projects/<key>.yaml)
	Rules    map[string]any // merged rules: general → project (no agent layer in this surface)
	Overview string         // contents of projects/<project>/README.md (empty if absent)
	Index    string         // contents of projects/<project>/INDEX.md (empty if absent)
	Sources  []string       // file paths actually read during the load (cite-anchor)
	LoadedAt time.Time      // wall-clock at load (for cite + staleness)
}

// Load assembles the project context from the canonical-store at canonRoot
// for the given project key. Missing files are treated as empty (not
// errors) so the load works on partial substrates — only YAML parse
// errors abort.
//
// Resolver layering: general → projects/<project>.yaml. The agent layer
// is intentionally NOT merged here (this is project context, not
// per-agent context); agents read their own per-agent YAML separately.
//
// Use LoadWithAgent when the caller wants the agent layer too
// (e.g., the sessionopen handler wraps the same primitives but
// includes the per-agent rules under the "agent" key).
func Load(canonRoot, project string) (*Context, error) {
	return LoadWithAgent(canonRoot, project, "")
}

// LoadWithAgent is Load with an explicit agent layer merged in. When
// agent is non-empty, the resolver also reads rules/agents/<agent>.yaml
// and exposes its content under the merged map's "agent" key (matching
// the resolver's existing semantic). When agent is empty, behaves
// identically to Load.
//
// Used by internal/agents/sessionopen so /api/session-open and the
// per-pivot context_load tool share the same Layer-2 loader.
func LoadWithAgent(canonRoot, project, agent string) (*Context, error) {
	if project == "" {
		return nil, fmt.Errorf("project key required")
	}

	merged, err := rules.Resolve(canonRoot, project, agent)
	if err != nil {
		return nil, fmt.Errorf("resolve rules: %w", err)
	}

	c := &Context{
		Project:  project,
		Rules:    merged,
		LoadedAt: time.Now().UTC(),
	}

	// Track which files contributed (for cite-anchor at the bottom of
	// the markdown render). Resolve doesn't expose this, so we
	// re-derive the candidate paths and stat them.
	candidates := []string{
		filepath.Join(canonRoot, "rules", "general.yaml"),
		filepath.Join(canonRoot, "projects", project+".yaml"),
		filepath.Join(canonRoot, "rules", "projects", project+".yaml"), // legacy fallback
	}
	for _, p := range candidates {
		if _, err := os.Stat(p); err == nil {
			c.Sources = append(c.Sources, p)
		}
	}

	// Read project library README + INDEX if present.
	overviewPath := filepath.Join(canonRoot, "projects", project, "README.md")
	if data, err := os.ReadFile(overviewPath); err == nil {
		c.Overview = string(data)
		c.Sources = append(c.Sources, overviewPath)
	}
	indexPath := filepath.Join(canonRoot, "projects", project, "INDEX.md")
	if data, err := os.ReadFile(indexPath); err == nil {
		c.Index = string(data)
		c.Sources = append(c.Sources, indexPath)
	}

	return c, nil
}

// Markdown renders the Context as a single markdown blob suitable for
// agent consumption (drop into Claude's context, the agent reads it
// like any system-context block). Stable section ordering so caches +
// diffs across loads are deterministic.
func (c *Context) Markdown() string {
	var b strings.Builder

	fmt.Fprintf(&b, "# Project context: %s\n\n", c.Project)
	fmt.Fprintf(&b, "_Loaded at %s_\n\n", c.LoadedAt.Format(time.RFC3339))

	// Resolved rules — render the top-level keys as collapsible sections.
	// We don't try to be fancy; YAML-style indentation keeps it readable.
	if len(c.Rules) > 0 {
		b.WriteString("## Resolved rules (general → project)\n\n")
		b.WriteString("```yaml\n")
		b.WriteString(renderYAMLLike(c.Rules, 0))
		b.WriteString("```\n\n")
	}

	if c.Overview != "" {
		b.WriteString("## Library overview\n\n")
		b.WriteString(c.Overview)
		if !strings.HasSuffix(c.Overview, "\n") {
			b.WriteString("\n")
		}
		b.WriteString("\n")
	}

	if c.Index != "" {
		b.WriteString("## Library index\n\n")
		b.WriteString(c.Index)
		if !strings.HasSuffix(c.Index, "\n") {
			b.WriteString("\n")
		}
		b.WriteString("\n")
	}

	if len(c.Sources) > 0 {
		b.WriteString("## Sources\n\n")
		for _, s := range c.Sources {
			fmt.Fprintf(&b, "- `%s`\n", s)
		}
	}

	return b.String()
}

// renderYAMLLike serializes a map[string]any to YAML-shaped text. Not a
// general YAML serializer — handles the shapes the rules resolver
// produces (nested maps + scalars + string slices). Sorted keys so
// output is deterministic.
func renderYAMLLike(m map[string]any, indent int) string {
	if len(m) == 0 {
		return ""
	}
	keys := make([]string, 0, len(m))
	for k := range m {
		keys = append(keys, k)
	}
	sort.Strings(keys)

	pad := strings.Repeat("  ", indent)
	var b strings.Builder
	for _, k := range keys {
		v := m[k]
		switch val := v.(type) {
		case map[string]any:
			fmt.Fprintf(&b, "%s%s:\n", pad, k)
			b.WriteString(renderYAMLLike(val, indent+1))
		case []any:
			fmt.Fprintf(&b, "%s%s:\n", pad, k)
			for _, item := range val {
				fmt.Fprintf(&b, "%s  - %v\n", pad, item)
			}
		case string:
			fmt.Fprintf(&b, "%s%s: %s\n", pad, k, quoteIfNeeded(val))
		default:
			fmt.Fprintf(&b, "%s%s: %v\n", pad, k, val)
		}
	}
	return b.String()
}

// quoteIfNeeded wraps strings containing yaml-special characters in
// double-quotes. Best-effort — we're producing markdown for human/agent
// consumption, not strict YAML.
func quoteIfNeeded(s string) string {
	if s == "" {
		return `""`
	}
	if strings.ContainsAny(s, ":#'\"\n") || strings.HasPrefix(s, "-") || strings.HasPrefix(s, "?") {
		// Escape embedded double quotes
		escaped := strings.ReplaceAll(s, `"`, `\"`)
		return `"` + escaped + `"`
	}
	return s
}

// BootstrapContext extends Context with the durable CL substrate fields
// agents need on resume. Per vision.md ("Agents are stateless; CL is
// durable"): the active phase scope-lock doc, ratchets ledger, per-agent
// last_state.json, and discipline-anchors.md together carry resume-state
// — replacing the prior "iterate hub_read until empty" backlog scrape.
type BootstrapContext struct {
	*Context
	Agent             string // agent identity (brian/rain/emma); empty if not requested
	PhaseDoc          string // contents of phase/<active>.md or "" if absent
	PhaseDocPath      string // path of the active phase doc (newest mtime under phase/)
	Ratchets          string // contents of ratchets/active.md
	LastState         string // contents of <agent>/last_state.json (raw JSON)
	DisciplineAnchors string // contents of <agent>/discipline-anchors.md
}

// LoadBootstrap loads the full CL bootstrap context per vision.md three-
// layer model (knowledge + discipline + state). Equivalent to
// LoadWithAgent (rules + project library) plus the per-agent resume
// anchors:
//   - phase/<active>.md — newest mtime under phase/ (active scope-lock doc)
//   - ratchets/active.md — operational ratchet ledger
//   - <agent>/last_state.json — R20 AgentState resume anchor
//   - <agent>/discipline-anchors.md — R24 mutual-halt anchor
//
// Missing files are treated as empty. Use Markdown() on the returned
// value for agent-consumption rendering.
func LoadBootstrap(canonRoot, project, agent string) (*BootstrapContext, error) {
	base, err := LoadWithAgent(canonRoot, project, agent)
	if err != nil {
		return nil, err
	}
	bc := &BootstrapContext{Context: base, Agent: agent}

	if path, body := findActivePhaseDoc(canonRoot); path != "" {
		bc.PhaseDoc = body
		bc.PhaseDocPath = path
		bc.Sources = append(bc.Sources, path)
	}

	rpath := filepath.Join(canonRoot, "ratchets", "active.md")
	if data, err := os.ReadFile(rpath); err == nil {
		bc.Ratchets = string(data)
		bc.Sources = append(bc.Sources, rpath)
	}

	if agent != "" {
		lspath := filepath.Join(canonRoot, agent, "last_state.json")
		if data, err := os.ReadFile(lspath); err == nil {
			bc.LastState = string(data)
			bc.Sources = append(bc.Sources, lspath)
		}
		dapath := filepath.Join(canonRoot, agent, "discipline-anchors.md")
		if data, err := os.ReadFile(dapath); err == nil {
			bc.DisciplineAnchors = string(data)
			bc.Sources = append(bc.Sources, dapath)
		}
	}

	return bc, nil
}

// findActivePhaseDoc returns (path, body) of the newest phase-*.md under
// canonRoot/phase/. Returns ("", "") if no phase doc exists. Newest by
// mtime so closed-phase snapshots don't shadow the live one.
func findActivePhaseDoc(canonRoot string) (string, string) {
	dir := filepath.Join(canonRoot, "phase")
	entries, err := os.ReadDir(dir)
	if err != nil {
		return "", ""
	}
	var newestPath string
	var newestMtime time.Time
	for _, e := range entries {
		if e.IsDir() || !strings.HasSuffix(e.Name(), ".md") {
			continue
		}
		info, err := e.Info()
		if err != nil {
			continue
		}
		if info.ModTime().After(newestMtime) {
			newestMtime = info.ModTime()
			newestPath = filepath.Join(dir, e.Name())
		}
	}
	if newestPath == "" {
		return "", ""
	}
	data, err := os.ReadFile(newestPath)
	if err != nil {
		return "", ""
	}
	return newestPath, string(data)
}

// Markdown renders the BootstrapContext as a single markdown blob with
// the durable-substrate sections appended after the standard project
// context. Stable section ordering for cache + diff determinism.
func (bc *BootstrapContext) Markdown() string {
	var b strings.Builder

	b.WriteString(bc.Context.Markdown())

	if bc.Agent != "" {
		fmt.Fprintf(&b, "\n## Bootstrap context (agent=%s)\n\n", bc.Agent)
		b.WriteString("_Per vision.md: agents are stateless; CL is durable. The sections below carry resume-state in lieu of hub-message backlog scraping._\n\n")
	}

	if bc.PhaseDoc != "" {
		fmt.Fprintf(&b, "### Active phase doc — `%s`\n\n", bc.PhaseDocPath)
		b.WriteString(bc.PhaseDoc)
		if !strings.HasSuffix(bc.PhaseDoc, "\n") {
			b.WriteString("\n")
		}
		b.WriteString("\n")
	}

	if bc.Ratchets != "" {
		b.WriteString("### Active ratchets — `ratchets/active.md`\n\n")
		b.WriteString(bc.Ratchets)
		if !strings.HasSuffix(bc.Ratchets, "\n") {
			b.WriteString("\n")
		}
		b.WriteString("\n")
	}

	if bc.LastState != "" {
		fmt.Fprintf(&b, "### Last state — `%s/last_state.json`\n\n", bc.Agent)
		b.WriteString("```json\n")
		b.WriteString(bc.LastState)
		if !strings.HasSuffix(bc.LastState, "\n") {
			b.WriteString("\n")
		}
		b.WriteString("```\n\n")
	}

	if bc.DisciplineAnchors != "" {
		fmt.Fprintf(&b, "### Discipline anchors — `%s/discipline-anchors.md`\n\n", bc.Agent)
		b.WriteString(bc.DisciplineAnchors)
		if !strings.HasSuffix(bc.DisciplineAnchors, "\n") {
			b.WriteString("\n")
		}
		b.WriteString("\n")
	}

	return b.String()
}
