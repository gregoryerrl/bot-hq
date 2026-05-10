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
