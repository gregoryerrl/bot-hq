// Package cl — fix.go: Phase Z S2 FixProject + FixAll.
//
// FixProject is a thin wrapper per plan-doc PB-5:
//
//   1. surgical-append `remote_url: ""` if absent (preserves yaml formatting
//      per plan-doc §5 + Plan-B OQ-1 — no marshal-reformat)
//   2. SeedProjectSubdirs(canonRoot, project, dirExts...) — canonical 9
//      + any extension *directories* (file-class extensions are not
//      seeded; they exist or they don't)
//   3. IndexProject(project) — regenerate INDEX.md
//
// The remote_url surgical-append is the one known yaml-normalization
// case (988 only, per plan-doc §1.2). yaml.v3 marshal-rewrite is avoided
// deliberately — it would reorder keys + normalize whitespace, breaking
// the "additive only, no formatting changes" constraint.

package cl

import (
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strings"

	"github.com/gregoryerrl/bot-hq/internal/projects"
	"gopkg.in/yaml.v3"
)

// remoteURLLineRE matches an existing top-level remote_url declaration.
// Anchored to line-start so commented-out lines do not count.
var remoteURLLineRE = regexp.MustCompile(`(?m)^remote_url:`)

// projectNameLineRE matches an existing top-level project_name
// declaration. Used as the anchor for surgical-append placement of
// remote_url: "" when remote_url is absent (places it just before
// project_name, matching the bot-hq.yaml + bcc-ad-manager.yaml
// convention where remote_url precedes project_name).
var projectNameLineRE = regexp.MustCompile(`(?m)^project_name:`)

// FixProject normalizes <canonRoot>/projects/<project>.yaml + seeds
// canonical-9-subdir + extension-dir layout + regenerates INDEX.md.
// Idempotent (re-running on a fully-fixed project is a no-op).
func (c *CL) FixProject(project string) error {
	if project == "" {
		return fmt.Errorf("fix: project required")
	}
	yamlPath := filepath.Join(c.root, "projects", project+".yaml")
	data, err := os.ReadFile(yamlPath)
	if err != nil {
		if os.IsNotExist(err) {
			return fmt.Errorf("%w: %s", ErrNotFound, yamlPath)
		}
		return fmt.Errorf("fix: read %s: %w", yamlPath, err)
	}

	fixed, changed := ensureRemoteURLPresent(data)
	if changed {
		if err := os.WriteFile(yamlPath, fixed, 0o644); err != nil {
			return fmt.Errorf("fix: write %s: %w", yamlPath, err)
		}
		data = fixed
	}

	var rules projects.Rules
	if err := yaml.Unmarshal(data, &rules); err != nil {
		return fmt.Errorf("fix: parse %s: %w", yamlPath, err)
	}

	dirExts := dirClassExtensions(rules.Extensions.AllNames())
	if err := SeedProjectSubdirs(c.root, project, dirExts...); err != nil {
		return fmt.Errorf("fix: seed: %w", err)
	}

	if _, _, err := c.IndexProject(project); err != nil {
		return fmt.Errorf("fix: re-index %s: %w", project, err)
	}
	return nil
}

// FixAll runs FixProject for every project under canonRoot/projects/.
// Returns the list of project names successfully fixed plus the first
// error encountered (remaining projects skipped on error — re-run after
// addressing the root cause).
func (c *CL) FixAll() ([]string, error) {
	projs, err := c.ListProjects()
	if err != nil {
		return nil, fmt.Errorf("fix-all: list projects: %w", err)
	}
	fixed := make([]string, 0, len(projs))
	for _, p := range projs {
		if err := c.FixProject(p); err != nil {
			return fixed, err
		}
		fixed = append(fixed, p)
	}
	return fixed, nil
}

// ensureRemoteURLPresent adds `remote_url: ""` at the top of the yaml
// when no remote_url key is declared. Returns the (possibly mutated)
// bytes + a bool flag for "changed". Placement: just before the
// project_name line (matching bot-hq.yaml + bcc-ad-manager.yaml
// convention). If no project_name line is found (malformed yaml or
// edge case), returns unmodified — caller surfaces via the subsequent
// yaml parse step.
func ensureRemoteURLPresent(data []byte) ([]byte, bool) {
	if remoteURLLineRE.Match(data) {
		return data, false
	}
	loc := projectNameLineRE.FindIndex(data)
	if loc == nil {
		return data, false
	}
	inject := []byte("remote_url: \"\"\n")
	out := make([]byte, 0, len(data)+len(inject))
	out = append(out, data[:loc[0]]...)
	out = append(out, inject...)
	out = append(out, data[loc[0]:]...)
	return out, true
}

// dirClassExtensions filters a flat extension basename list to those
// without a `.` — per plan-doc PB-1 filename convention, dir-class
// extensions are bare-name (e.g., "phase", "ratchets") and file-class
// extensions carry an explicit extension (e.g., "vision.md"). Only
// dir-class extensions are passed to SeedProjectSubdirs.
func dirClassExtensions(names []string) []string {
	out := make([]string, 0, len(names))
	for _, n := range names {
		if n == "" {
			continue
		}
		if strings.Contains(n, ".") {
			continue
		}
		out = append(out, n)
	}
	return out
}
