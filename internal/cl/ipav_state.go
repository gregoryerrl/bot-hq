// Package cl — ipav_state.go: T-1.7 IPAV-state-tracking schema in CL.
//
// Wraps the MVT IPAV state-machine (internal/mvt) with CL-managed persistence.
// CL is the single-source-of-truth for IPAV per-task state per phase-t.md v5.
//
// CL.IPAVState(project, taskID) is the typed helper that resolves the
// canonical path + reads + YAML-decodes into an *mvt.TaskState. Use
// CL.SaveIPAVState to persist updates atomically.
//
// File path: ~/.bot-hq/projects/<project>/tasks/<task-id>/ipav-state.yaml

package cl

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"

	"github.com/gregoryerrl/bot-hq/internal/mvt"
	"gopkg.in/yaml.v3"
)

// IPAVState loads + decodes an IPAV per-task state from CL.
// Returns ErrNotFound if the file does not exist.
func (c *CL) IPAVState(project, taskID string) (*mvt.TaskState, error) {
	if project == "" || taskID == "" {
		return nil, errors.New("project and taskID are required")
	}
	path := filepath.Join(c.root, "projects", project, "tasks", taskID, "ipav-state.yaml")
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

// SaveIPAVState persists an IPAV per-task state to CL with atomic-write.
// Project + TaskID are derived from the canonical-path convention.
func (c *CL) SaveIPAVState(project string, ts *mvt.TaskState) error {
	if project == "" {
		return errors.New("project is required")
	}
	if ts == nil || ts.TaskID == "" {
		return errors.New("TaskState requires TaskID")
	}
	path := filepath.Join(c.root, "projects", project, "tasks", ts.TaskID, "ipav-state.yaml")
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return fmt.Errorf("mkdir: %w", err)
	}
	return ts.Save(path)
}

// ListIPAVStates enumerates all IPAV state files for the given project.
// Returns nil + nil error when project directory does not exist.
func (c *CL) ListIPAVStates(project string) ([]*mvt.TaskState, error) {
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
		ts, err := c.IPAVState(project, taskID)
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

// ListProjectsWithIPAVTasks returns all project IDs that have at least one
// IPAV task state file. Convenience for cross-project enumeration.
func (c *CL) ListProjectsWithIPAVTasks() ([]string, error) {
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

// IPAVStatePath returns the canonical path for a per-task IPAV state file.
// Convenience wrapper exposing the path-convention to external callers
// (e.g. R49 pre-seal-audit hook).
func (c *CL) IPAVStatePath(project, taskID string) string {
	return filepath.Join(c.root, "projects", project, "tasks", taskID, "ipav-state.yaml")
}

