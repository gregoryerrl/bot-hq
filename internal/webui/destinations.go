// Package webui — destination-allowlist nav model per Phase N v3.x-1
// scope-lock-v4.2 (Form Y default). Replaces the v3b walkCanonicalTree
// denylist with a curated nav of named "destinations" — each destination
// is a user-recognizable knowledge surface (Documents, Sessions, Rules,
// Plans, etc.) resolved to filesystem paths via a Resolver.
//
// Two sections:
//   - Global: 8 destinations, project-independent (Documents, Sessions,
//     Disciplines, Ratchets, Global Rules, Agent Rules, Agent Notes, Etc).
//   - Per-project: 4 destinations parameterized by project key (Overview,
//     Rules, Plans, Etc).
//
// Bot-hq is the just-another-project — its artifacts live at top-level
// (phase/, ratchets/, gates/, README.md) rather than under projects/bot-hq/.
// Special-case resolvers handle bot-hq paths; everything else uses the
// projects/<p>/* convention.
//
// HIDE list (per scope-lock-v4.2): runtime/code/log/binary surfaces that
// fail the agent-context-yes / internal-code-no discriminator. Enforced
// implicitly: the allowlist only ever surfaces md/yaml from named dirs,
// so .db/.log/.jsonl/.json-runtime/source-code/voice-mirror-log/per-agent
// last_state.json/diag/sentinels/bridge/plugins-github never appear.
package webui

import (
	"errors"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

// Destination is a curated nav entry. The Resolver computes the TreeNode
// list for the active project (ignored for Section=="global"). Section is
// "global" or "project".
type Destination struct {
	Name     string `json:"name"`
	Section  string `json:"section"`
	Resolver func(canonRoot, project string) ([]TreeNode, error) `json:"-"`
	// Nodes populated by ResolveDestinations; Resolver remains the source.
	Nodes []TreeNode `json:"nodes,omitempty"`
}

// Project is a registered project key. bot-hq is always present; others
// are discovered from projects/*.yaml.
type Project struct {
	Name string `json:"name"`
}

// ListProjects returns the registered project list (alphabetical, with
// bot-hq always first). Discovers projects via projects/*.yaml; missing
// dir returns just bot-hq.
func ListProjects(canonRoot string) ([]Project, error) {
	out := []Project{{Name: "bot-hq"}}
	dir := filepath.Join(canonRoot, "projects")
	entries, err := os.ReadDir(dir)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return out, nil
		}
		return nil, err
	}
	var rest []string
	for _, e := range entries {
		if e.IsDir() {
			continue
		}
		name := e.Name()
		if !strings.HasSuffix(name, ".yaml") {
			continue
		}
		key := strings.TrimSuffix(name, ".yaml")
		if key == "bot-hq" {
			continue // already first
		}
		rest = append(rest, key)
	}
	sort.Strings(rest)
	for _, k := range rest {
		out = append(out, Project{Name: k})
	}
	return out, nil
}

// GlobalDestinations returns the 8 global-section destinations in the
// canonical scope-lock-v4.2 order.
func GlobalDestinations() []Destination {
	return []Destination{
		{Name: "Documents", Section: "global", Resolver: resolveDocuments},
		{Name: "Sessions", Section: "global", Resolver: resolveSessions},
		{Name: "Disciplines", Section: "global", Resolver: resolveDisciplines},
		{Name: "Ratchets", Section: "global", Resolver: resolveRatchets},
		{Name: "Global Rules", Section: "global", Resolver: resolveGlobalRules},
		{Name: "Agent Rules", Section: "global", Resolver: resolveAgentRules},
		{Name: "Agent Notes", Section: "global", Resolver: resolveAgentNotes},
		{Name: "Etc", Section: "global", Resolver: resolveGlobalEtc},
	}
}

// ProjectDestinations returns the 4 per-project destinations for project.
func ProjectDestinations() []Destination {
	return []Destination{
		{Name: "Overview", Section: "project", Resolver: resolveProjectOverview},
		{Name: "Rules", Section: "project", Resolver: resolveProjectRules},
		{Name: "Plans", Section: "project", Resolver: resolveProjectPlans},
		{Name: "Etc", Section: "project", Resolver: resolveProjectEtc},
	}
}

// ResolveDestinations runs every Resolver for the given project (project
// is ignored by global resolvers). Returns the populated destination list
// in the canonical order: 8 global, then 4 project. Errors from any
// individual resolver are surfaced; missing files yield empty Nodes
// (with a Missing TreeNode marker for the Overview blank-state case).
func ResolveDestinations(canonRoot, project string) ([]Destination, error) {
	all := append(GlobalDestinations(), ProjectDestinations()...)
	for i := range all {
		nodes, err := all[i].Resolver(canonRoot, project)
		if err != nil {
			return nil, err
		}
		all[i].Nodes = nodes
	}
	return all, nil
}

// --- single-file helpers ----------------------------------------------

// fileNode emits a single-file TreeNode for relPath under root. Returns
// nil (and no error) if file missing — most resolvers silently elide
// missing files. Pass missingMarker=true to emit a "Missing: true" sentinel
// node instead (used by Overview blank-state).
func fileNode(root, relPath string, missingMarker bool) (*TreeNode, error) {
	abs := filepath.Join(root, relPath)
	info, err := os.Stat(abs)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			if missingMarker {
				return &TreeNode{
					Path:    relPath,
					Name:    filepath.Base(relPath),
					Type:    "file",
					Missing: true,
				}, nil
			}
			return nil, nil
		}
		return nil, err
	}
	if info.IsDir() {
		return nil, nil
	}
	return &TreeNode{
		Path:  relPath,
		Name:  filepath.Base(relPath),
		Type:  "file",
		Mtime: info.ModTime().UTC().Format("2006-01-02T15:04:05Z"),
		Size:  info.Size(),
	}, nil
}

// dirFiles emits TreeNodes for every immediate-child file under root/relDir
// matching exts (case-sensitive; ".md" / ".yaml"). Skips dotfiles and
// subdirs. Sorted alphabetical. Missing dir returns empty list (no error).
func dirFiles(root, relDir string, exts ...string) ([]TreeNode, error) {
	abs := filepath.Join(root, relDir)
	entries, err := os.ReadDir(abs)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return nil, nil
		}
		return nil, err
	}
	var out []TreeNode
	for _, e := range entries {
		name := e.Name()
		if strings.HasPrefix(name, ".") {
			continue
		}
		if e.IsDir() {
			continue
		}
		if len(exts) > 0 {
			ok := false
			ext := filepath.Ext(name)
			for _, want := range exts {
				if ext == want {
					ok = true
					break
				}
			}
			if !ok {
				continue
			}
		}
		info, err := e.Info()
		if err != nil {
			return nil, err
		}
		rel := name
		if relDir != "" {
			rel = relDir + "/" + name
		}
		out = append(out, TreeNode{
			Path:  rel,
			Name:  name,
			Type:  "file",
			Mtime: info.ModTime().UTC().Format("2006-01-02T15:04:05Z"),
			Size:  info.Size(),
		})
	}
	sort.SliceStable(out, func(i, j int) bool { return out[i].Name < out[j].Name })
	return out, nil
}

// --- global resolvers --------------------------------------------------

func resolveDocuments(root, _ string) ([]TreeNode, error) {
	n, err := fileNode(root, "README.md", false)
	if err != nil || n == nil {
		return nil, err
	}
	return []TreeNode{*n}, nil
}

func resolveSessions(root, _ string) ([]TreeNode, error) {
	// Walk sessions/<id>/manifest.md — each session-dir gets one entry.
	dir := filepath.Join(root, "sessions")
	entries, err := os.ReadDir(dir)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return nil, nil
		}
		return nil, err
	}
	var out []TreeNode
	for _, e := range entries {
		if !e.IsDir() {
			continue
		}
		name := e.Name()
		if strings.HasPrefix(name, ".") {
			continue
		}
		rel := "sessions/" + name + "/manifest.md"
		n, err := fileNode(root, rel, false)
		if err != nil {
			return nil, err
		}
		if n == nil {
			continue
		}
		// Surface session-id as the display name for clarity.
		n.Name = name
		out = append(out, *n)
	}
	sort.SliceStable(out, func(i, j int) bool { return out[i].Name > out[j].Name }) // newest first
	return out, nil
}

func resolveDisciplines(root, _ string) ([]TreeNode, error) {
	n, err := fileNode(root, "discipline-log.md", false)
	if err != nil || n == nil {
		return nil, err
	}
	return []TreeNode{*n}, nil
}

func resolveRatchets(root, _ string) ([]TreeNode, error) {
	return dirFiles(root, "ratchets", ".md")
}

func resolveGlobalRules(root, _ string) ([]TreeNode, error) {
	// scope-lock-v4.2 line 697: rules/general/ subdir is empty; surface
	// the single-file rules/general.yaml only.
	n, err := fileNode(root, "rules/general.yaml", false)
	if err != nil || n == nil {
		return nil, err
	}
	return []TreeNode{*n}, nil
}

func resolveAgentRules(root, _ string) ([]TreeNode, error) {
	return dirFiles(root, "rules/agents", ".yaml")
}

func resolveAgentNotes(root, _ string) ([]TreeNode, error) {
	var out []TreeNode
	for _, agent := range []string{"brian", "rain"} {
		rel := agent + "/discipline-anchors.md"
		n, err := fileNode(root, rel, false)
		if err != nil {
			return nil, err
		}
		if n != nil {
			n.Name = agent + "/discipline-anchors.md"
			out = append(out, *n)
		}
	}
	return out, nil
}

func resolveGlobalEtc(_, _ string) ([]TreeNode, error) {
	// Future-extensible cross-project catch-all. Empty for now.
	return nil, nil
}

// --- per-project resolvers --------------------------------------------

func resolveProjectOverview(root, project string) ([]TreeNode, error) {
	if project == "" {
		return nil, nil
	}
	if project == "bot-hq" {
		// Bot-hq overview is the top-level README.md (already surfaced by
		// Documents in the global section, but also belongs as Overview).
		n, err := fileNode(root, "README.md", false)
		if err != nil || n == nil {
			return nil, err
		}
		return []TreeNode{*n}, nil
	}
	rel := "projects/" + project + "/overview.md"
	// Blank-state when missing: surface a Missing-marker so the frontend
	// can offer "create overview" affordance per scope-lock S1 stub.
	n, err := fileNode(root, rel, true)
	if err != nil || n == nil {
		return nil, err
	}
	return []TreeNode{*n}, nil
}

func resolveProjectRules(root, project string) ([]TreeNode, error) {
	if project == "" {
		return nil, nil
	}
	var out []TreeNode
	// projects/<p>.yaml for all projects
	if n, err := fileNode(root, "projects/"+project+".yaml", false); err != nil {
		return nil, err
	} else if n != nil {
		out = append(out, *n)
	}
	// bot-hq additionally surfaces gates/*.md (per user msg 8889 "gates are
	// rules"; gate-content folds into rules.yaml gates: schema-field in Phase O).
	if project == "bot-hq" {
		gates, err := dirFiles(root, "gates", ".md")
		if err != nil {
			return nil, err
		}
		out = append(out, gates...)
	}
	return out, nil
}

func resolveProjectPlans(root, project string) ([]TreeNode, error) {
	if project == "" {
		return nil, nil
	}
	if project == "bot-hq" {
		// Phases ARE bot-hq's plans (user msg 8859).
		return dirFiles(root, "phase", ".md")
	}
	return dirFiles(root, "projects/"+project+"/plans", ".md")
}

func resolveProjectEtc(root, project string) ([]TreeNode, error) {
	if project == "" || project == "bot-hq" {
		// bot-hq: empty-today (future-extensible).
		return nil, nil
	}
	var out []TreeNode
	for _, sub := range []string{"clips", "eod"} {
		rel := "projects/" + project + "/" + sub
		nodes, err := dirFiles(root, rel, ".md")
		if err != nil {
			return nil, err
		}
		out = append(out, nodes...)
	}
	return out, nil
}
