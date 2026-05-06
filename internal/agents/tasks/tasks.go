// Package tasks implements per-project task-list parsing per Phase N v3.x-2
// design-spike §2.5+§3 (component C5). Tasks live at
// ~/.bot-hq/projects/<p>/tasks.md with a YAML frontmatter task-list and
// optional free-form markdown body.
//
// Status vocabulary: pending | in_progress | done | blocked. Schema validation
// is best-effort (warns on unknown status; doesn't error). Forward-compat
// fields preserved through round-trip.
package tasks

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"gopkg.in/yaml.v3"
)

// Task is one entry in the tasks frontmatter.
type Task struct {
	ID     string `yaml:"id" json:"id"`
	Title  string `yaml:"title" json:"title"`
	Status string `yaml:"status" json:"status"` // pending | in_progress | done | blocked
	Owner  string `yaml:"owner,omitempty" json:"owner,omitempty"`
}

// Frontmatter is the YAML head of tasks.md.
type Frontmatter struct {
	Tasks []Task `yaml:"tasks"`
}

// File is the parsed tasks.md.
type File struct {
	Frontmatter Frontmatter
	Body        string
}

// ValidStatuses lists the canonical status vocabulary.
var ValidStatuses = []string{"pending", "in_progress", "done", "blocked"}

// Path returns the on-disk path for project p's tasks.md.
func Path(canonRoot, project string) string {
	return filepath.Join(canonRoot, "projects", project, "tasks.md")
}

// Read parses the tasks.md for project. Returns (nil, nil) when absent.
func Read(canonRoot, project string) (*File, error) {
	p := Path(canonRoot, project)
	data, err := os.ReadFile(p)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return nil, nil
		}
		return nil, err
	}
	return Decode(string(data))
}

// Decode parses raw on-disk content into File. Tolerates missing
// frontmatter by treating the whole file as body (no tasks).
func Decode(raw string) (*File, error) {
	f := &File{}
	if !strings.HasPrefix(raw, "---\n") && !strings.HasPrefix(raw, "---\r\n") {
		f.Body = raw
		return f, nil
	}
	rest := raw[4:]
	if strings.HasPrefix(raw, "---\r\n") {
		rest = raw[5:]
	}
	end := strings.Index(rest, "\n---")
	if end < 0 {
		f.Body = raw
		return f, nil
	}
	fm := rest[:end]
	body := rest[end+4:]
	body = strings.TrimPrefix(body, "\n")
	body = strings.TrimPrefix(body, "\r\n")

	if err := yaml.Unmarshal([]byte(fm), &f.Frontmatter); err != nil {
		return nil, fmt.Errorf("parse tasks frontmatter: %w", err)
	}
	f.Body = body
	return f, nil
}

// Write atomically writes f to the tasks.md path for project.
func Write(canonRoot, project string, f File) error {
	if project == "" {
		return errors.New("project required")
	}
	dir := filepath.Join(canonRoot, "projects", project)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return err
	}
	final := Path(canonRoot, project)
	tmp := final + ".tmp"
	var buf strings.Builder
	buf.WriteString("---\n")
	fmBytes, err := yaml.Marshal(f.Frontmatter)
	if err != nil {
		return err
	}
	buf.Write(fmBytes)
	buf.WriteString("---\n\n")
	buf.WriteString(f.Body)
	if !strings.HasSuffix(f.Body, "\n") {
		buf.WriteString("\n")
	}
	if err := os.WriteFile(tmp, []byte(buf.String()), 0o644); err != nil {
		return err
	}
	if err := os.Rename(tmp, final); err != nil {
		_ = os.Remove(tmp)
		return err
	}
	return nil
}

// Validate returns warnings (not errors) for tasks with non-vocabulary
// statuses. Schema-permissive per Q-rules-6 forward-compat.
func (f *File) Validate() []string {
	var warns []string
	for _, t := range f.Frontmatter.Tasks {
		if !isValidStatus(t.Status) {
			warns = append(warns, fmt.Sprintf("task %q has non-vocabulary status %q (allowed: %v)", t.ID, t.Status, ValidStatuses))
		}
		if t.ID == "" {
			warns = append(warns, fmt.Sprintf("task with title %q missing id", t.Title))
		}
	}
	return warns
}

func isValidStatus(s string) bool {
	for _, v := range ValidStatuses {
		if v == s {
			return true
		}
	}
	return false
}
