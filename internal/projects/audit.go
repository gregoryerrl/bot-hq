package projects

import (
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"gopkg.in/yaml.v3"
)

// flatFormKeys lists the legacy flat-form yaml keys per Phase N v3.x-1.5
// design-spike §2.1. A canonical (post-migration) per-project YAML must
// not contain any of these — they have been replaced by their nested
// equivalents under gates/branch/commit.
var flatFormKeys = []string{
	"push_requires_approval",
	"force_push_blocked",
	"force_push_token_format",
	"branch_pattern",
	"branch_examples",
	"branch_pattern_help",
	"coder_tools_blocked",
	"coder_tools_per_action_approval",
	"commit_style",
	"require_issue_link",
}

// AuditStatus is the per-file canonical-audit verdict.
type AuditStatus string

const (
	StatusCanonical AuditStatus = "CANONICAL"
	StatusDrift     AuditStatus = "DRIFT"
	StatusError     AuditStatus = "ERROR"
)

// AuditResult bundles a single per-file audit outcome.
type AuditResult struct {
	Path       string      `json:"path"`
	Status     AuditStatus `json:"status"`
	FlatKeys   []string    `json:"flat_keys,omitempty"`
	ParseError string      `json:"parse_error,omitempty"`
}

// AuditCanonical inspects every ~/.bot-hq/projects/*.yaml file under the
// supplied projects directory and reports whether each is in canonical
// nested form (Phase N v3.x-1.5 §2.1) or contains legacy flat-form
// keys awaiting migration. Pure read-only — does not modify any files.
//
// Phase O drain item: complements the read-side dual-form unmarshaler
// (Rules.UnmarshalYAML) + write-side normalizer (projects.Normalize) by
// providing an audit primitive callers (CLI / future scheduled audit /
// pre-deploy check) can invoke without touching content.
//
// Returns sorted-by-path AuditResult slice. Caller (CLI) renders.
func AuditCanonical(projectsDir string) ([]AuditResult, error) {
	entries, err := os.ReadDir(projectsDir)
	if err != nil {
		return nil, fmt.Errorf("read projects dir %s: %w", projectsDir, err)
	}

	var results []AuditResult
	for _, e := range entries {
		if e.IsDir() || !strings.HasSuffix(e.Name(), ".yaml") {
			continue
		}
		path := filepath.Join(projectsDir, e.Name())
		results = append(results, auditOne(path))
	}
	sort.Slice(results, func(i, j int) bool { return results[i].Path < results[j].Path })
	return results, nil
}

// auditOne checks a single file. Pure: does not write, does not touch
// other files. Errors during read/parse → StatusError with ParseError
// populated; canonical form → StatusCanonical; flat-form keys present
// → StatusDrift with FlatKeys listing which.
func auditOne(path string) AuditResult {
	r := AuditResult{Path: path}
	data, err := os.ReadFile(path)
	if err != nil {
		r.Status = StatusError
		r.ParseError = fmt.Sprintf("read: %v", err)
		return r
	}
	var raw map[string]any
	if err := yaml.Unmarshal(data, &raw); err != nil {
		r.Status = StatusError
		r.ParseError = fmt.Sprintf("yaml parse: %v", err)
		return r
	}
	for _, k := range flatFormKeys {
		if _, present := raw[k]; present {
			r.FlatKeys = append(r.FlatKeys, k)
		}
	}
	if len(r.FlatKeys) > 0 {
		r.Status = StatusDrift
	} else {
		r.Status = StatusCanonical
	}
	return r
}

// FormatAuditResults renders a sorted result slice as a human-readable
// report (one line per file + summary). Used by the CLI subcommand.
func FormatAuditResults(results []AuditResult) string {
	var b strings.Builder
	canonCount, driftCount, errCount := 0, 0, 0
	for _, r := range results {
		switch r.Status {
		case StatusCanonical:
			canonCount++
			fmt.Fprintf(&b, "  %s: CANONICAL\n", r.Path)
		case StatusDrift:
			driftCount++
			fmt.Fprintf(&b, "  %s: DRIFT (flat keys: %s)\n", r.Path, strings.Join(r.FlatKeys, ", "))
		case StatusError:
			errCount++
			fmt.Fprintf(&b, "  %s: ERROR (%s)\n", r.Path, r.ParseError)
		}
	}
	total := len(results)
	fmt.Fprintf(&b, "\n%d files audited: %d canonical / %d drift / %d error\n", total, canonCount, driftCount, errCount)
	if driftCount > 0 {
		fmt.Fprintf(&b, "DRIFT files have legacy flat-form keys; will be normalized on next save via webui (PUT /api/files/projects/<p>.yaml).\n")
	}
	return b.String()
}
