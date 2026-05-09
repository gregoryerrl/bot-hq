// Package faulttree implements the cl_fault_tree investigator-tool
// per phase-t.md v5 T-2.4 + R44 BILATERAL-INVESTIGATION-DISCIPLINE
// fault-tree-bridge artifact.
//
// Apollo RCA-style fault-tree with two node-types:
//
//	action — a hypothesis or causal action that may or may not be confirmed
//	condition — a precondition or contributing factor
//
// Fault-trees serve as the bilateral-bridge artifact at 50% sync-point
// (per R44 expanded). Navigators hold the tree + assign leaves to drivers;
// drivers run Zeller hypothesis loops on assigned leaves (cl_hypothesis_loop).
//
// Storage: per-task at `~/.bot-hq/projects/<project>/tasks/<task-id>/fault-tree.json`.
// Atomic-write via .tmp + rename.

package faulttree

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"time"

	"github.com/google/uuid"
)

// NodeType discriminates Apollo RCA node-types.
type NodeType string

const (
	NodeAction    NodeType = "action"
	NodeCondition NodeType = "condition"
)

// NodeStatus tracks whether a node is open (unresolved), confirmed, or refuted.
type NodeStatus string

const (
	StatusOpen      NodeStatus = "open"
	StatusConfirmed NodeStatus = "confirmed"
	StatusRefuted   NodeStatus = "refuted"
	StatusWeak      NodeStatus = "weak" // partial-evidence; needs upgrade-or-refute
)

// Node is one entry in the fault-tree.
type Node struct {
	ID            string     `json:"id"`         // generated UUID
	Type          NodeType   `json:"type"`
	Title         string     `json:"title"`
	Description   string     `json:"description,omitempty"`
	Status        NodeStatus `json:"status"`
	Owner         string     `json:"owner,omitempty"`         // agent-id (R44 anti-cross: navigator owns; driver investigates)
	Investigator  string     `json:"investigator,omitempty"`  // agent-id assigned to investigate (must != Owner per R44)
	CiteAnchors   []string   `json:"cite_anchors,omitempty"`  // evidence references (msg-id / file-path)
	ParentID      string     `json:"parent_id,omitempty"`     // parent node-id; empty = root
	ChildrenIDs   []string   `json:"children_ids,omitempty"`  // ordered list of child node-ids
	CreatedAt     time.Time  `json:"created_at"`
	UpdatedAt     time.Time  `json:"updated_at"`
}

// Tree is the fault-tree document.
type Tree struct {
	TaskID    string  `json:"task_id"`
	RootID    string  `json:"root_id,omitempty"`
	Nodes     []*Node `json:"nodes"`
	CreatedAt time.Time `json:"created_at"`
	UpdatedAt time.Time `json:"updated_at"`
}

// NewTree initializes an empty fault-tree for a task.
func NewTree(taskID string) *Tree {
	now := time.Now().UTC()
	return &Tree{
		TaskID:    taskID,
		Nodes:     []*Node{},
		CreatedAt: now,
		UpdatedAt: now,
	}
}

// AddNode appends a new node + returns its assigned ID. ParentID can be
// empty for root; otherwise must match an existing node's ID.
func (t *Tree) AddNode(n *Node) (string, error) {
	if n == nil {
		return "", errors.New("nil node")
	}
	if n.Type != NodeAction && n.Type != NodeCondition {
		return "", fmt.Errorf("invalid node type: %s", n.Type)
	}
	if n.Title == "" {
		return "", errors.New("title is required")
	}
	if n.Status == "" {
		n.Status = StatusOpen
	}
	now := time.Now().UTC()
	n.ID = uuid.New().String()
	n.CreatedAt = now
	n.UpdatedAt = now

	if n.ParentID != "" {
		parent := t.findNode(n.ParentID)
		if parent == nil {
			return "", fmt.Errorf("parent %s not found", n.ParentID)
		}
		parent.ChildrenIDs = append(parent.ChildrenIDs, n.ID)
		parent.UpdatedAt = now
	} else if t.RootID == "" {
		t.RootID = n.ID
	}

	t.Nodes = append(t.Nodes, n)
	t.UpdatedAt = now
	return n.ID, nil
}

// SetStatus updates a node's status with timestamp. Returns ErrNodeNotFound
// if id does not match.
func (t *Tree) SetStatus(id string, status NodeStatus) error {
	n := t.findNode(id)
	if n == nil {
		return ErrNodeNotFound
	}
	n.Status = status
	n.UpdatedAt = time.Now().UTC()
	t.UpdatedAt = n.UpdatedAt
	return nil
}

// AssignInvestigator assigns an investigator to a node per R44 anti-cross
// strong-style discipline. Returns error if investigator == Owner (would
// violate R44 BILATERAL-INVESTIGATION anti-confirmation-bias).
func (t *Tree) AssignInvestigator(id, investigator string) error {
	n := t.findNode(id)
	if n == nil {
		return ErrNodeNotFound
	}
	if investigator == "" {
		return errors.New("investigator is required")
	}
	if investigator == n.Owner {
		return fmt.Errorf("R44 anti-cross violation: investigator %q == owner; cannot drive own hypothesis", investigator)
	}
	n.Investigator = investigator
	n.UpdatedAt = time.Now().UTC()
	t.UpdatedAt = n.UpdatedAt
	return nil
}

// AddCiteAnchor appends an evidence cite-anchor to a node.
func (t *Tree) AddCiteAnchor(id, anchor string) error {
	n := t.findNode(id)
	if n == nil {
		return ErrNodeNotFound
	}
	n.CiteAnchors = append(n.CiteAnchors, anchor)
	n.UpdatedAt = time.Now().UTC()
	t.UpdatedAt = n.UpdatedAt
	return nil
}

// findNode is the internal lookup helper.
func (t *Tree) findNode(id string) *Node {
	for _, n := range t.Nodes {
		if n.ID == id {
			return n
		}
	}
	return nil
}

// GetNode returns a node by ID or nil if not found.
func (t *Tree) GetNode(id string) *Node {
	return t.findNode(id)
}

// LeafNodes returns all nodes with no children (terminal nodes ready for
// driver investigation per R44 hypothesis-investigation phase).
func (t *Tree) LeafNodes() []*Node {
	var leaves []*Node
	for _, n := range t.Nodes {
		if len(n.ChildrenIDs) == 0 {
			leaves = append(leaves, n)
		}
	}
	return leaves
}

// OpenLeafNodes returns all leaves with status == open or weak (need
// further investigation).
func (t *Tree) OpenLeafNodes() []*Node {
	var open []*Node
	for _, leaf := range t.LeafNodes() {
		if leaf.Status == StatusOpen || leaf.Status == StatusWeak {
			open = append(open, leaf)
		}
	}
	return open
}

// IsConvergence returns true when all confirmed nodes have ≥2 cite-anchors,
// no nodes are weak, and no leaves are open. Per R44 hypothesis-set
// convergence criterion.
func (t *Tree) IsConvergence() bool {
	for _, n := range t.Nodes {
		if n.Status == StatusOpen || n.Status == StatusWeak {
			return false
		}
		if n.Status == StatusConfirmed && len(n.CiteAnchors) < 2 {
			return false
		}
	}
	return true
}

// Save persists the tree to the canonical path with atomic-write.
func (t *Tree) Save(path string) error {
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return fmt.Errorf("mkdir: %w", err)
	}
	data, err := json.MarshalIndent(t, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal: %w", err)
	}
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, data, 0o644); err != nil {
		return fmt.Errorf("write tmp: %w", err)
	}
	return os.Rename(tmp, path)
}

// Load reads + decodes a fault-tree from the given path.
func Load(path string) (*Tree, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, ErrTreeNotFound
		}
		return nil, fmt.Errorf("read: %w", err)
	}
	var t Tree
	if err := json.Unmarshal(data, &t); err != nil {
		return nil, fmt.Errorf("unmarshal: %w", err)
	}
	return &t, nil
}

// CanonicalPath returns the canonical fault-tree path for a task.
func CanonicalPath(homeDir, project, taskID string) string {
	return filepath.Join(homeDir, ".bot-hq", "projects", project, "tasks", taskID, "fault-tree.json")
}

// Sentinel errors.
var (
	ErrNodeNotFound = errors.New("node not found")
	ErrTreeNotFound = errors.New("fault-tree not found")
)
