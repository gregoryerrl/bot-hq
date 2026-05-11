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
// Post-Z-1: bot-hq is just-another-project. Its operational artifacts
// (phase/, ratchets/, discipline-log.md, voice-mirror-log.md, arcs-index,
// conventions-index) live under projects/bot-hq/ following the same
// projects/<p>/* convention as every other project. Cross-project
// surfaces (gates/, rules/, sessions/, agent-state) stay at top-level.
//
// Global nav resolvers (Disciplines + Ratchets) are bot-hq-canonical
// minimum-fix: they resolve to projects/bot-hq/* explicitly. Full
// per-project parameterization is a Phase Z+ scope.
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

// ProjectDestinations returns the per-project destinations for project.
// Phase Q library schema: Overview/Rules/Plans + Architecture/Decisions/
// Conventions/Glossary/Audit notes (the per-project library subdirs)
// + EOD/Clips (split out from the v3.x-1 Etc catch-all) + Project docs
// (dual-root: project's own ~/Projects/<p>/docs/, read-only).
func ProjectDestinations() []Destination {
	return []Destination{
		{Name: "Overview", Section: "project", Resolver: resolveProjectOverview},
		{Name: "Rules", Section: "project", Resolver: resolveProjectRules},
		{Name: "Plans", Section: "project", Resolver: resolveProjectPlans},
		{Name: "Architecture", Section: "project", Resolver: resolveProjectArchitecture},
		{Name: "Decisions", Section: "project", Resolver: resolveProjectDecisions},
		{Name: "Conventions", Section: "project", Resolver: resolveProjectConventions},
		{Name: "Glossary", Section: "project", Resolver: resolveProjectGlossary},
		{Name: "Audit notes", Section: "project", Resolver: resolveProjectAuditNotes},
		{Name: "EOD", Section: "project", Resolver: resolveProjectEOD},
		{Name: "Clips", Section: "project", Resolver: resolveProjectClips},
		{Name: "Project docs", Section: "project", Resolver: resolveProjectExternalDocs},
	}
}

// ResolveDestinations runs every Resolver for the given project (project
// is ignored by global resolvers). Returns the populated destination list
// in the canonical order: 8 global, then per-project. Errors from any
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
	var out []TreeNode
	for _, rel := range []string{"README.md", "tasks.md"} {
		n, err := fileNode(root, rel, false)
		if err != nil {
			return nil, err
		}
		if n != nil {
			out = append(out, *n)
		}
	}
	return out, nil
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
	// Post-Z-1: bot-hq's discipline-log lives at projects/bot-hq/discipline-log.md
	// (canonical project; Phase Z+ may parameterize per current-project context).
	n, err := fileNode(root, "projects/bot-hq/discipline-log.md", false)
	if err != nil || n == nil {
		return nil, err
	}
	return []TreeNode{*n}, nil
}

func resolveRatchets(root, _ string) ([]TreeNode, error) {
	// Post-Z-1: bot-hq's ratchets/ dir lives at projects/bot-hq/ratchets/
	// (canonical project; Phase Z+ may parameterize per current-project context).
	return dirFiles(root, "projects/bot-hq/ratchets", ".md")
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
	// Phase Q library schema: README.md + INDEX.md at projects/<p>/.
	// All registered projects (incl. bot-hq) get the same shape.
	var out []TreeNode
	for _, rel := range []string{
		"projects/" + project + "/README.md",
		"projects/" + project + "/INDEX.md",
	} {
		n, err := fileNode(root, rel, false)
		if err != nil {
			return nil, err
		}
		if n != nil {
			out = append(out, *n)
		}
	}
	if len(out) > 0 {
		return out, nil
	}
	// Blank-state when both missing: surface a Missing-marker for the
	// README so the frontend offers the "create overview" affordance.
	n, err := fileNode(root, "projects/"+project+"/README.md", true)
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
	// Note: gates/ stays top-level post-Z-1 (cross-project discipline).
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
		// Phases ARE bot-hq's plans (user msg 8859). Post-Z-1: phase/ moved
		// from top-level to projects/bot-hq/phase/ per CL generalization.
		return dirFiles(root, "projects/bot-hq/phase", ".md")
	}
	return dirFiles(root, "projects/"+project+"/plans", ".md")
}

// Phase Q library schema resolvers — each surfaces a single subdir under
// projects/<p>/. dirFiles returns empty (no error) when the dir doesn't
// exist, so unscaffolded projects show "(empty)" rather than failing.

func resolveProjectArchitecture(root, project string) ([]TreeNode, error) {
	if project == "" {
		return nil, nil
	}
	return dirFiles(root, "projects/"+project+"/architecture", ".md")
}

func resolveProjectDecisions(root, project string) ([]TreeNode, error) {
	if project == "" {
		return nil, nil
	}
	return dirFiles(root, "projects/"+project+"/decisions", ".md")
}

func resolveProjectConventions(root, project string) ([]TreeNode, error) {
	if project == "" {
		return nil, nil
	}
	return dirFiles(root, "projects/"+project+"/conventions", ".md")
}

func resolveProjectGlossary(root, project string) ([]TreeNode, error) {
	if project == "" {
		return nil, nil
	}
	return dirFiles(root, "projects/"+project+"/glossary", ".md")
}

func resolveProjectAuditNotes(root, project string) ([]TreeNode, error) {
	if project == "" {
		return nil, nil
	}
	return dirFiles(root, "projects/"+project+"/audit-notes", ".md")
}

func resolveProjectEOD(root, project string) ([]TreeNode, error) {
	if project == "" {
		return nil, nil
	}
	return dirFiles(root, "projects/"+project+"/eod", ".md")
}

func resolveProjectClips(root, project string) ([]TreeNode, error) {
	if project == "" {
		return nil, nil
	}
	return dirFiles(root, "projects/"+project+"/clips", ".md")
}

// resolveProjectExternalDocs implements the dual-root surface — read-only
// view of the project's own docs/ subdir at ~/Projects/<project>/docs/.
// Out-of-tree (not under canonical root); the frontend treats these as
// non-editable. Empty for projects without a docs/ dir at the expected
// path.
func resolveProjectExternalDocs(_, project string) ([]TreeNode, error) {
	if project == "" {
		return nil, nil
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return nil, err
	}
	docsRoot := filepath.Join(home, "Projects", project, "docs")
	return externalDirFiles(docsRoot, project, ".md")
}

// externalDirFiles emits read-only TreeNodes for every immediate-child
// file under absDir matching exts. Path is set to "external/<project>/
// <basename>" so frontend can route to the /api/external-file endpoint.
// Missing dir returns empty (no error).
func externalDirFiles(absDir, project string, exts ...string) ([]TreeNode, error) {
	entries, err := os.ReadDir(absDir)
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
		out = append(out, TreeNode{
			Path:     "external/" + project + "/" + name,
			Name:     name,
			Type:     "file",
			Mtime:    info.ModTime().UTC().Format("2006-01-02T15:04:05Z"),
			Size:     info.Size(),
			External: true,
		})
	}
	sort.SliceStable(out, func(i, j int) bool { return out[i].Name < out[j].Name })
	return out, nil
}
