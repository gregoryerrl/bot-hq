// Package cl — ipiv_state.go: T-1.7 IPIV-state-tracking schema in CL.
//
// Wraps the MVT IPIV state-machine (internal/mvt) with CL-managed persistence.
// CL is the single-source-of-truth for IPIV per-task state per phase-t.md v5.
//
// CL.IPIVState(project, taskID) is the typed helper that resolves the
// canonical path + reads + YAML-decodes into an *mvt.TaskState. Use
// CL.SaveIPIVState to persist updates atomically.
//
// File path: ~/.bot-hq/projects/<project>/tasks/<task-id>/ipiv-state.yaml

package cl

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/gregoryerrl/bot-hq/internal/mvt"
	"gopkg.in/yaml.v3"
)

// IPIVState loads + decodes an IPIV per-task state from CL.
// Returns ErrNotFound if the file does not exist.
func (c *CL) IPIVState(project, taskID string) (*mvt.TaskState, error) {
	if project == "" || taskID == "" {
		return nil, errors.New("project and taskID are required")
	}
	path := filepath.Join(c.root, "projects", project, "tasks", taskID, "ipiv-state.yaml")
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, fmt.Errorf("%w: %s", ErrNotFound, path)
		}
		return nil, fmt.Errorf("read %s: %w", path, err)
	}
	var ts mvt.TaskState
	if err := yaml.Unmarshal(data, &ts); err != nil {
		return nil, fmt.Errorf("unmarshal %s: %w", path, err)
	}
	return &ts, nil
}

// SaveIPIVState persists an IPIV per-task state to CL with atomic-write.
// Project + TaskID are derived from the canonical-path convention.
func (c *CL) SaveIPIVState(project string, ts *mvt.TaskState) error {
	if project == "" {
		return errors.New("project is required")
	}
	if ts == nil || ts.TaskID == "" {
		return errors.New("TaskState requires TaskID")
	}
	path := filepath.Join(c.root, "projects", project, "tasks", ts.TaskID, "ipiv-state.yaml")
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return fmt.Errorf("mkdir: %w", err)
	}
	return ts.Save(path)
}

// ListIPIVStates enumerates all IPIV state files for the given project.
// Returns nil + nil error when project directory does not exist.
func (c *CL) ListIPIVStates(project string) ([]*mvt.TaskState, error) {
	if project == "" {
		return nil, errors.New("project is required")
	}
	tasksDir := filepath.Join(c.root, "projects", project, "tasks")
	entries, err := os.ReadDir(tasksDir)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, fmt.Errorf("readdir %s: %w", tasksDir, err)
	}

	var states []*mvt.TaskState
	for _, e := range entries {
		if !e.IsDir() {
			continue
		}
		taskID := e.Name()
		ts, err := c.IPIVState(project, taskID)
		if err != nil {
			if errors.Is(err, ErrNotFound) {
				continue
			}
			return nil, fmt.Errorf("load task %s: %w", taskID, err)
		}
		states = append(states, ts)
	}
	return states, nil
}

// ListProjectsWithIPIVTasks returns all project IDs that have at least one
// IPIV task state file. Convenience for cross-project enumeration.
func (c *CL) ListProjectsWithIPIVTasks() ([]string, error) {
	projectsDir := filepath.Join(c.root, "projects")
	entries, err := os.ReadDir(projectsDir)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, fmt.Errorf("readdir %s: %w", projectsDir, err)
	}

	var projects []string
	for _, e := range entries {
		if !e.IsDir() {
			continue
		}
		project := e.Name()
		// Skip projects without a tasks/ subdir
		tasksDir := filepath.Join(projectsDir, project, "tasks")
		if _, err := os.Stat(tasksDir); err != nil {
			continue
		}
		// Only include projects with at least one task subdir
		taskEntries, _ := os.ReadDir(tasksDir)
		hasTask := false
		for _, te := range taskEntries {
			if te.IsDir() {
				hasTask = true
				break
			}
		}
		if hasTask {
			projects = append(projects, project)
		}
	}
	return projects, nil
}

// IPIVStatePath returns the canonical path for a per-task IPIV state file.
// Convenience wrapper exposing the path-convention to external callers
// (e.g. R49 pre-seal-audit hook).
func (c *CL) IPIVStatePath(project, taskID string) string {
	return filepath.Join(c.root, "projects", project, "tasks", taskID, "ipiv-state.yaml")
}

// projectFromIPIVPath extracts the project-id from a canonical IPIV state path.
// Returns empty string if path doesn't match the expected layout.
// Used for back-derivation in tests + cross-ref-graph.
func (c *CL) projectFromIPIVPath(path string) string {
	rel, err := filepath.Rel(c.root, path)
	if err != nil {
		return ""
	}
	parts := strings.Split(rel, string(filepath.Separator))
	if len(parts) < 4 || parts[0] != "projects" || parts[2] != "tasks" {
		return ""
	}
	return parts[1]
}
