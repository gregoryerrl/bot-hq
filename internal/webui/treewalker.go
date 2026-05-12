package webui

import (
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/gregoryerrl/bot-hq/internal/cl"
	"github.com/gregoryerrl/bot-hq/internal/projects"
	"gopkg.in/yaml.v3"
)

// BuildFilteredTree walks <canonRoot>/<rootRel> applying the canonical
// filter chain (HIDE per cl.HiddenPaths → extensions allowlist from the
// project yaml → catch-all "project_private"). Hidden entries are dropped;
// surviving entries carry their classification on TreeNode.Class.
//
// rootRel is canonical-store-relative (forward-slash, no leading "/").
// Empty rootRel walks from canonRoot itself.
//
// Per plan-doc §3 S4 (cl-uniformity Phase R3 R5): no file ever disappears
// from the response unless cl.IsHidden returns true — undeclared on-disk
// content surfaces as project_private (PB-Q1 invariant).
func BuildFilteredTree(canonRoot, rootRel string) ([]TreeNode, error) {
	if canonRoot == "" {
		return nil, fmt.Errorf("BuildFilteredTree: canonRoot required")
	}
	rootRel = strings.TrimPrefix(filepath.ToSlash(rootRel), "/")
	rootRel = strings.TrimSuffix(rootRel, "/")
	absRoot := filepath.Join(canonRoot, rootRel)
	classifier := newExtensionsClassifier(canonRoot, rootRel)
	return walkFiltered(rootRel, absRoot, classifier)
}

// extensionsClassifier maps a top-level (under projects/<p>/) basename to
// the extensions-allowlist class it falls under. project is the project
// key derived from rootRel; empty if walk is not under projects/<p>/.
type extensionsClassifier struct {
	project string
	byName  map[string]string
}

func newExtensionsClassifier(canonRoot, rootRel string) *extensionsClassifier {
	c := &extensionsClassifier{byName: map[string]string{}}
	parts := strings.Split(rootRel, "/")
	if len(parts) >= 2 && parts[0] == "projects" && parts[1] != "" {
		c.project = parts[1]
	}
	if c.project == "" {
		return c
	}
	yamlPath := filepath.Join(canonRoot, "projects", c.project+".yaml")
	data, err := os.ReadFile(yamlPath)
	if err != nil {
		return c
	}
	var rules projects.Rules
	if err := yaml.Unmarshal(data, &rules); err != nil {
		return c
	}
	for _, n := range rules.Extensions.UniversalOptIn {
		c.byName[n] = "universal_opt_in"
	}
	for _, n := range rules.Extensions.ExternalDocsPointer {
		c.byName[n] = "external_docs_pointer"
	}
	for _, n := range rules.Extensions.BrainDuoOperational {
		c.byName[n] = "brain_duo_operational"
	}
	for _, n := range rules.Extensions.FoundationalAnchors {
		c.byName[n] = "foundational_anchors"
	}
	return c
}

// classifyName returns the allowlisted class for a basename inside the
// project subtree, or "" if no allowlist match.
func (c *extensionsClassifier) classifyName(name string) string {
	if c == nil || len(c.byName) == 0 {
		return ""
	}
	return c.byName[name]
}

// walkFiltered recursively builds TreeNode children for absPath using
// cl.IsHidden + the classifier. relPrefix is the canonical-store-relative
// prefix for child paths (e.g., "projects/bot-hq" makes a child node carry
// "projects/bot-hq/INDEX.md").
func walkFiltered(relPrefix, absPath string, classifier *extensionsClassifier) ([]TreeNode, error) {
	entries, err := os.ReadDir(absPath)
	if err != nil {
		return nil, err
	}
	out := make([]TreeNode, 0, len(entries))
	for _, e := range entries {
		name := e.Name()
		isDir := e.IsDir()
		var entryRel string
		if relPrefix == "" {
			entryRel = name
		} else {
			entryRel = relPrefix + "/" + name
		}
		if cl.IsHidden(entryRel, isDir) {
			continue
		}
		info, err := e.Info()
		if err != nil {
			continue
		}
		class := classifyForEntry(entryRel, classifier)
		node := TreeNode{
			Path:  entryRel,
			Name:  name,
			Mtime: info.ModTime().UTC().Format("2006-01-02T15:04:05Z"),
			Class: class,
		}
		if isDir {
			node.Type = "dir"
			children, err := walkFiltered(entryRel, filepath.Join(absPath, name), classifier)
			if err == nil {
				node.Children = children
			}
		} else {
			node.Type = "file"
			node.Size = info.Size()
		}
		out = append(out, node)
	}
	sort.Slice(out, func(i, j int) bool {
		if out[i].Type != out[j].Type {
			return out[i].Type == "dir"
		}
		return out[i].Name < out[j].Name
	})
	return out, nil
}

// classifyForEntry resolves the filter-chain class for a single entry
// inside projects/<p>/. The class is taken from the third path component
// (e.g., projects/bot-hq/architecture/sessions-as-containers.md → class
// "architecture"); deeper descendants inherit their class from the same
// component so cross-project queries find the whole subtree.
//
// Returns "" for entries not under projects/<p>/ (global walk has no
// classifier). Returns "project_private" as the catch-all for declared-
// nowhere on-disk content inside projects/<p>/ (PB-Q1 invariant: no file
// disappears).
func classifyForEntry(entryRel string, classifier *extensionsClassifier) string {
	if classifier == nil || classifier.project == "" {
		return ""
	}
	parts := strings.Split(entryRel, "/")
	if len(parts) < 3 || parts[0] != "projects" || parts[1] != classifier.project {
		return ""
	}
	third := parts[2]
	if isCanonicalSubdir(third) {
		return third
	}
	if third == "README.md" || third == "INDEX.md" {
		return "overview"
	}
	if cls := classifier.classifyName(third); cls != "" {
		return cls
	}
	return "project_private"
}

// isCanonicalSubdir returns true for the 9 canonical subdirs that every
// per-project library is expected to have (per Phase Q INDEX schema +
// Phase R cl-uniformity scope).
func isCanonicalSubdir(name string) bool {
	switch name {
	case "architecture", "audit-notes", "clips", "conventions",
		"decisions", "eod", "glossary", "plans", "tasks":
		return true
	}
	return false
}
