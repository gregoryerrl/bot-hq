// Package cl is the Canonical Library (CL) programmatic API per phase-t.md
// v5 T-1.6. CL is the user-facing name for the ~/.bot-hq/ context library
// (internal Go-code identifier: canonical-store). This package exposes
// CL artifacts via Go API for consumption by:
//
//   - T-1.7 IPIV-state-tracking schema in CL
//   - T-1.8 cross-reference graph
//   - T-1.10 discoverability + indexing
//   - T-2 investigator-toolset (cluster D + E)
//   - T-2 R49 PRE-SEAL-MECHANICAL-AUDIT (R44/R49 hooks)
//
// CL artifacts include:
//   - phase/<phase-id>.md             — scope-lock-doc per R10
//   - ratchets/active.md              — forward-ratchet ledger
//   - projects/<project>/             — per-project library
//   - rules/{general,projects,agents}/ — discipline-rule configs
//   - sessions/<id>/                  — session manifests
//   - gates/<checklist>.md            — Tier-3 pre-action checklists
//   - <agent>/{last_state.json,discipline-anchors.md}
//   - discipline-log.md / tasks.md / voice-mirror-log.md
//   - 10 top-level reference docs (glossary / roles / etc.)
//
// Excluded from CL (per Phase R R3-b conventions):
//   - code (lives in ~/Projects/bot-hq/)
//   - agent-memory (~/.claude/projects/.../memory/)
//   - runtime ephemera (hub.db / live.log / debug.log)
package cl

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

// Class enumerates artifact classes within CL.
type Class string

const (
	ClassPhase          Class = "phase"
	ClassRatchet        Class = "ratchet"
	ClassProject        Class = "project"
	ClassRule           Class = "rule"
	ClassSession        Class = "session"
	ClassGate           Class = "gate"
	ClassAgentState     Class = "agent-state"
	ClassDisciplineLog  Class = "discipline-log"
	ClassTasks          Class = "tasks"
	ClassReference      Class = "reference"
	ClassIPIVState      Class = "ipiv-state" // T-1.7 sub-class
	ClassUnknown        Class = "unknown"
)

// Artifact is one CL entity loaded from disk.
type Artifact struct {
	Class    Class
	ID       string // class-relative identifier (e.g. "phase-t" / "active" / "bot-hq")
	Path     string // absolute path on disk
	Content  []byte // raw file content (lazily loaded by Get/Read; nil until requested)
	Project  string // optional: project-id when class=ClassProject sub-artifacts
	Loaded   bool   // true if Content has been read
}

// CL is the programmatic API entry-point. Construct via NewCL with the
// canonical-store root path (defaults to ~/.bot-hq/ when empty).
type CL struct {
	root string
}

// NewCL constructs a CL handle. If root is empty, uses ~/.bot-hq/ via
// $HOME expansion. Returns error if root cannot be resolved.
func NewCL(root string) (*CL, error) {
	if root == "" {
		home, err := os.UserHomeDir()
		if err != nil {
			return nil, fmt.Errorf("home dir: %w", err)
		}
		root = filepath.Join(home, ".bot-hq")
	}
	info, err := os.Stat(root)
	if err != nil {
		return nil, fmt.Errorf("cl root %s: %w", root, err)
	}
	if !info.IsDir() {
		return nil, fmt.Errorf("cl root %s is not a directory", root)
	}
	return &CL{root: root}, nil
}

// Root returns the absolute path of the CL root directory.
func (c *CL) Root() string { return c.root }

// PathFor constructs the canonical file path for a (class, id) pair.
// Returns ErrUnsupportedClass for classes lacking a single-file convention.
func (c *CL) PathFor(class Class, id string) (string, error) {
	switch class {
	case ClassPhase:
		return filepath.Join(c.root, "phase", id+".md"), nil
	case ClassRatchet:
		if id == "active" {
			return filepath.Join(c.root, "ratchets", "active.md"), nil
		}
		// Closed ratchets stored as date-stamped files
		return filepath.Join(c.root, "ratchets", id+".md"), nil
	case ClassGate:
		return filepath.Join(c.root, "gates", id+".md"), nil
	case ClassDisciplineLog:
		return filepath.Join(c.root, "discipline-log.md"), nil
	case ClassTasks:
		return filepath.Join(c.root, "tasks.md"), nil
	case ClassReference:
		return filepath.Join(c.root, id+".md"), nil
	case ClassAgentState:
		return filepath.Join(c.root, id, "last_state.json"), nil
	case ClassProject:
		// id format: "<project>/README" or "<project>/INDEX" or just "<project>" → README
		if !strings.Contains(id, "/") {
			id = id + "/README"
		}
		return filepath.Join(c.root, "projects", id+".md"), nil
	case ClassRule:
		// id format: "general" or "projects/<proj>" or "agents/<agent>"
		return filepath.Join(c.root, "rules", id+".yaml"), nil
	case ClassSession:
		return filepath.Join(c.root, "sessions", id, "manifest.md"), nil
	case ClassIPIVState:
		// id format: "<project>/<task-id>"
		return filepath.Join(c.root, "projects", id, "ipiv-state.yaml"), nil
	default:
		return "", fmt.Errorf("%w: %s", ErrUnsupportedClass, class)
	}
}

// Get loads an artifact by (class, id). Returns ErrNotFound if the file does not exist.
// The returned Artifact has Content populated + Loaded=true.
func (c *CL) Get(class Class, id string) (*Artifact, error) {
	path, err := c.PathFor(class, id)
	if err != nil {
		return nil, err
	}
	return c.Read(path)
}

// Read loads an artifact directly from a file path. Class is auto-detected
// from the path location. Returns ErrNotFound if the file does not exist.
func (c *CL) Read(path string) (*Artifact, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, fmt.Errorf("%w: %s", ErrNotFound, path)
		}
		return nil, fmt.Errorf("read %s: %w", path, err)
	}
	a := &Artifact{
		Class:   c.detectClass(path),
		ID:      c.deriveID(path),
		Path:    path,
		Content: data,
		Loaded:  true,
		Project: c.deriveProject(path),
	}
	return a, nil
}

// Write persists an artifact to its canonical path. Creates parent directory
// if needed. Atomic-write via .tmp + rename. Sets Loaded=true on success.
func (c *CL) Write(a *Artifact) error {
	if a == nil || a.Path == "" {
		return errors.New("artifact requires Path")
	}
	if err := os.MkdirAll(filepath.Dir(a.Path), 0o755); err != nil {
		return fmt.Errorf("mkdir: %w", err)
	}
	tmp := a.Path + ".tmp"
	if err := os.WriteFile(tmp, a.Content, 0o644); err != nil {
		return fmt.Errorf("write tmp: %w", err)
	}
	if err := os.Rename(tmp, a.Path); err != nil {
		return fmt.Errorf("rename: %w", err)
	}
	a.Loaded = true
	return nil
}

// List enumerates artifacts of the given class. Returns empty slice on
// missing class-directory (treats as zero-result, not error).
func (c *CL) List(class Class) ([]*Artifact, error) {
	dir, err := c.classDir(class)
	if err != nil {
		return nil, err
	}
	entries, err := os.ReadDir(dir)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, fmt.Errorf("readdir %s: %w", dir, err)
	}

	var arts []*Artifact
	for _, e := range entries {
		if e.IsDir() {
			continue
		}
		path := filepath.Join(dir, e.Name())
		if !c.matchesClass(class, path) {
			continue
		}
		arts = append(arts, &Artifact{
			Class:   class,
			ID:      c.deriveID(path),
			Path:    path,
			Project: c.deriveProject(path),
		})
	}
	sort.Slice(arts, func(i, j int) bool { return arts[i].ID < arts[j].ID })
	return arts, nil
}

// Walk visits every CL artifact under the root, calling visit for each.
// Excludes runtime-ephemera (hub.db, *.log, *.tmp) + non-CL paths
// (~/.bot-hq/agents excluded; per-agent state via ClassAgentState only).
func (c *CL) Walk(visit func(*Artifact) error) error {
	return filepath.Walk(c.root, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}
		if info.IsDir() {
			return nil
		}
		if c.isRuntimeEphemera(path) {
			return nil
		}
		a := &Artifact{
			Class:   c.detectClass(path),
			ID:      c.deriveID(path),
			Path:    path,
			Project: c.deriveProject(path),
		}
		if a.Class == ClassUnknown {
			return nil
		}
		return visit(a)
	})
}

// LoadAgentState convenience helper: reads + JSON-decodes an agent's
// last_state.json into a generic map. T-1.7 IPIV-state-tracking will add
// a typed variant.
func (c *CL) LoadAgentState(agentID string) (map[string]interface{}, error) {
	a, err := c.Get(ClassAgentState, agentID)
	if err != nil {
		return nil, err
	}
	var state map[string]interface{}
	if err := json.Unmarshal(a.Content, &state); err != nil {
		return nil, fmt.Errorf("parse last_state.json: %w", err)
	}
	return state, nil
}

func (c *CL) classDir(class Class) (string, error) {
	switch class {
	case ClassPhase:
		return filepath.Join(c.root, "phase"), nil
	case ClassRatchet:
		return filepath.Join(c.root, "ratchets"), nil
	case ClassGate:
		return filepath.Join(c.root, "gates"), nil
	case ClassReference:
		return c.root, nil
	default:
		return "", fmt.Errorf("%w: list not supported for %s", ErrUnsupportedClass, class)
	}
}

func (c *CL) matchesClass(class Class, path string) bool {
	rel, err := filepath.Rel(c.root, path)
	if err != nil {
		return false
	}
	switch class {
	case ClassPhase:
		return strings.HasPrefix(rel, "phase/") && strings.HasSuffix(rel, ".md")
	case ClassRatchet:
		return strings.HasPrefix(rel, "ratchets/") && strings.HasSuffix(rel, ".md")
	case ClassGate:
		return strings.HasPrefix(rel, "gates/") && strings.HasSuffix(rel, ".md")
	case ClassReference:
		// Top-level *.md files are reference docs (glossary.md / roles.md / etc.)
		return !strings.Contains(rel, "/") && strings.HasSuffix(rel, ".md")
	}
	return false
}

func (c *CL) detectClass(path string) Class {
	rel, err := filepath.Rel(c.root, path)
	if err != nil {
		return ClassUnknown
	}
	parts := strings.Split(rel, string(filepath.Separator))
	if len(parts) == 0 {
		return ClassUnknown
	}
	switch parts[0] {
	case "phase":
		return ClassPhase
	case "ratchets":
		return ClassRatchet
	case "gates":
		return ClassGate
	case "rules":
		return ClassRule
	case "sessions":
		return ClassSession
	case "projects":
		if len(parts) > 2 && strings.HasSuffix(parts[len(parts)-1], "ipiv-state.yaml") {
			return ClassIPIVState
		}
		return ClassProject
	case "discipline-log.md":
		return ClassDisciplineLog
	case "tasks.md":
		return ClassTasks
	case "brian", "rain", "emma", "clive":
		// Per-agent state directories
		if len(parts) > 1 && parts[len(parts)-1] == "last_state.json" {
			return ClassAgentState
		}
		return ClassUnknown
	default:
		// Top-level reference docs (glossary.md / roles.md / etc.)
		if len(parts) == 1 && strings.HasSuffix(parts[0], ".md") {
			return ClassReference
		}
	}
	return ClassUnknown
}

func (c *CL) deriveID(path string) string {
	rel, err := filepath.Rel(c.root, path)
	if err != nil {
		return ""
	}
	parts := strings.Split(rel, string(filepath.Separator))
	if len(parts) == 0 {
		return ""
	}
	switch parts[0] {
	case "phase", "ratchets", "gates":
		// strip extension
		fname := parts[len(parts)-1]
		return strings.TrimSuffix(fname, filepath.Ext(fname))
	case "rules":
		// "rules/general.yaml" → "general"; "rules/projects/foo.yaml" → "projects/foo"
		joined := filepath.Join(parts[1:]...)
		return strings.TrimSuffix(joined, filepath.Ext(joined))
	case "sessions":
		// "sessions/<id>/manifest.md" → "<id>"
		if len(parts) > 1 {
			return parts[1]
		}
	case "projects":
		if len(parts) > 1 {
			// Strip extension on final segment
			parts[len(parts)-1] = strings.TrimSuffix(parts[len(parts)-1], filepath.Ext(parts[len(parts)-1]))
			return strings.Join(parts[1:], "/")
		}
	case "brian", "rain", "emma", "clive":
		return parts[0]
	default:
		fname := parts[len(parts)-1]
		return strings.TrimSuffix(fname, filepath.Ext(fname))
	}
	return rel
}

func (c *CL) deriveProject(path string) string {
	rel, err := filepath.Rel(c.root, path)
	if err != nil {
		return ""
	}
	parts := strings.Split(rel, string(filepath.Separator))
	if len(parts) >= 2 && parts[0] == "projects" {
		return parts[1]
	}
	return ""
}

func (c *CL) isRuntimeEphemera(path string) bool {
	base := filepath.Base(path)
	if strings.HasPrefix(base, "hub.db") {
		return true
	}
	if strings.HasSuffix(base, ".log") || strings.HasSuffix(base, ".tmp") {
		return true
	}
	if base == "bot-hq.db" {
		return true
	}
	rel, err := filepath.Rel(c.root, path)
	if err == nil {
		// Skip per-agent runtime ephemera + bridge/diag/sentinels
		if strings.HasPrefix(rel, "bridge/") || strings.HasPrefix(rel, "diag/") || strings.HasPrefix(rel, "sentinels/") {
			return true
		}
	}
	return false
}

// Sentinel errors.
var (
	ErrNotFound         = errors.New("artifact not found")
	ErrUnsupportedClass = errors.New("unsupported class")
)
