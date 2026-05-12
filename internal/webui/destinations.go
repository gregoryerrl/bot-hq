// Package webui — Project registry helper.
//
// File historically owned the destination-allowlist nav model (Phase N
// v3.x-1 form Y). Phase R3 R5 cl-uniformity-webui-nav-refactor migrated
// the per-project nav to a yaml-driven tree-walker (treewalker.go +
// crossproject.go); S4 dropped the resolveProject* fan-out and kept
// /api/destinations as a thin backward-compat shim. S5 (this slice)
// deletes the route + shim atomically alongside the frontend migration
// to /api/files?tree=1 — `Destination` type + `ResolveDestinations`
// helper went out with it.
//
// `Project` + `ListProjects` survive — they're still consumed by
// /api/projects, /api/cross-project, register-project flow, and CL's
// own ListProjects helper. Filename retained for blame-history continuity
// (rename deferred to a follow-on cleanup).
package webui

import (
	"errors"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

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
