// Package webui — Project registry + thin backward-compat shim for the
// destination-allowlist nav model (Phase N v3.x-1 form Y).
//
// Phase R3 R5 cl-uniformity-webui-nav-refactor migrated the per-project
// nav to a yaml-driven tree-walker (treewalker.go + crossproject.go);
// the 25+ resolveProject* functions previously in this file dropped.
// /api/destinations route + handleDestinations stay alive as a thin
// shim returning an empty Destination list — the route is removed by
// S5 alongside the frontend migration (per plan §2.3 corrected
// sequencing: deletion sequenced to S5 to avoid an S4-to-S5 404 window).
package webui

import (
	"errors"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

// Destination is preserved for /api/destinations response-shape backward
// compatibility. Post-S4 the Nodes field is always empty; consumers should
// migrate to /api/files?tree=1 via S5.
type Destination struct {
	Name    string     `json:"name"`
	Section string     `json:"section"`
	Nodes   []TreeNode `json:"nodes,omitempty"`
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
			continue
		}
		rest = append(rest, key)
	}
	sort.Strings(rest)
	for _, k := range rest {
		out = append(out, Project{Name: k})
	}
	return out, nil
}

// ResolveDestinations is the backward-compat shim for /api/destinations.
// Post-Phase-R3-R5 S4 returns an empty list — destination resolution
// moved to BuildFilteredTree + handleCrossProject. The route is kept
// alive through the S4-to-S5 window so the frontend doesn't 404 before
// it migrates to /api/files?tree=1.
func ResolveDestinations(_, _ string) ([]Destination, error) {
	return []Destination{}, nil
}
