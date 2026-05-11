// Package cl — validate.go: Phase Z S2 ValidateProject + ValidationIssue.
//
// ValidateProject loads <canonRoot>/projects/<project>.yaml and runs the
// extension-schema validation rules per plan-doc §1.1. Returns a flat
// slice of ValidationIssue; caller filters by Severity.
//
// Wraps projects.Rules.ValidateExtensions; adds the yaml-parse-failure
// case as a synthetic ValidationIssue rather than a hard error so that
// `cl-index --validate` can report all issues in a single pass.

package cl

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"

	"github.com/gregoryerrl/bot-hq/internal/projects"
	"gopkg.in/yaml.v3"
)

// ValidationIssue is one finding from ValidateProject. Severity is
// "error" or "warning". Path points at the offending file (yaml or
// declared extension target); empty when not applicable.
type ValidationIssue struct {
	Project  string
	Class    string
	Name     string
	Severity string
	Rule     string
	Path     string
}

func (i ValidationIssue) String() string {
	if i.Path != "" {
		return fmt.Sprintf("[%s] %s/%s/%q at %s: %s", i.Severity, i.Project, i.Class, i.Name, i.Path, i.Rule)
	}
	if i.Name != "" {
		return fmt.Sprintf("[%s] %s/%s/%q: %s", i.Severity, i.Project, i.Class, i.Name, i.Rule)
	}
	return fmt.Sprintf("[%s] %s: %s", i.Severity, i.Project, i.Rule)
}

// ValidateProject loads <canonRoot>/projects/<project>.yaml + emits
// ValidationIssue rows per plan-doc §1.1 rules. ErrNotFound is returned
// when the yaml file itself is absent; all other parse / semantic
// problems surface as ValidationIssue (severity error).
func (c *CL) ValidateProject(project string) ([]ValidationIssue, error) {
	if project == "" {
		return nil, fmt.Errorf("validate: project required")
	}
	yamlPath := filepath.Join(c.root, "projects", project+".yaml")
	data, err := os.ReadFile(yamlPath)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, fmt.Errorf("%w: %s", ErrNotFound, yamlPath)
		}
		return nil, fmt.Errorf("validate: read %s: %w", yamlPath, err)
	}

	var rules projects.Rules
	if err := yaml.Unmarshal(data, &rules); err != nil {
		return []ValidationIssue{{
			Project:  project,
			Severity: "error",
			Rule:     fmt.Sprintf("yaml parse failed: %v", err),
			Path:     yamlPath,
		}}, nil
	}

	out := make([]ValidationIssue, 0, 4)
	for _, e := range rules.ValidateExtensions(c.root) {
		issue := ValidationIssue{
			Project:  e.Project,
			Class:    e.Class,
			Name:     e.Name,
			Severity: e.Severity,
			Rule:     e.Rule,
		}
		if issue.Project == "" {
			issue.Project = project
		}
		if e.Severity == "warning" && e.Name != "" {
			issue.Path = filepath.Join(c.root, "projects", project, e.Name)
		}
		out = append(out, issue)
	}
	return out, nil
}

// ValidateAll runs ValidateProject for every project under canonRoot/projects/.
// Errors loading individual projects are returned as synthetic issues so a
// single missing yaml doesn't mask other findings.
func (c *CL) ValidateAll() ([]ValidationIssue, error) {
	projs, err := c.ListProjects()
	if err != nil {
		return nil, fmt.Errorf("validate-all: list projects: %w", err)
	}
	var all []ValidationIssue
	for _, p := range projs {
		issues, err := c.ValidateProject(p)
		if err != nil {
			if errors.Is(err, ErrNotFound) {
				all = append(all, ValidationIssue{
					Project:  p,
					Severity: "warning",
					Rule:     fmt.Sprintf("yaml absent (project dir exists at projects/%s/ without rules file)", p),
				})
				continue
			}
			return nil, err
		}
		all = append(all, issues...)
	}
	return all, nil
}
