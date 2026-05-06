package webui

import (
	"errors"
	"io/fs"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

// TreeNode is a single entry in the canonical-store tree response. Either
// File (with Mtime + Size) or Dir (with Children).
type TreeNode struct {
	Path     string     `json:"path"` // canonical-store-relative
	Name     string     `json:"name"` // basename
	Type     string     `json:"type"` // "file" or "dir"
	Mtime    string     `json:"mtime,omitempty"`
	Size     int64      `json:"size,omitempty"`
	Children []TreeNode `json:"children,omitempty"`
}

// canonicalSkipList is the set of relative paths excluded from the tree
// per Q7 LOCKED + scope-lock §v3a.5 §3.2 read-endpoint exclusions:
//
//   - <agent>/ — runtime state, not canonical-store
//   - gates/ — bot-hq runtime config
//   - hub.db, live.log — runtime state
//   - sessions/ — surfaced via /api/sessions, not /api/files
//
// Project-specific runtime state (e.g., projects/<p>/.git/) is also
// skipped via dotfile-skip below.
var canonicalSkipList = map[string]struct{}{
	"hub.db":         {},
	"live.log":       {},
	"gates":          {},
	"sessions":       {},
	"webui-index.db": {},
}

// canonicalSkipAgentDirs lists per-agent dirs to skip (each agent has a
// dir at top level e.g. brian/, rain/, emma/, clive/).
var canonicalSkipAgentDirs = map[string]struct{}{
	"brian": {},
	"rain":  {},
	"emma":  {},
	"clive": {},
}

// shouldSkip returns true for entries that aren't canonical-store class.
// Top-level only; nested paths are evaluated by their components in walk.
func shouldSkip(name string, isDir bool) bool {
	if strings.HasPrefix(name, ".") {
		return true // dotfiles + .git/ etc.
	}
	if _, ok := canonicalSkipList[name]; ok {
		return true
	}
	if isDir {
		if _, ok := canonicalSkipAgentDirs[name]; ok {
			return true
		}
	}
	return false
}

// walkCanonicalTree returns the tree rooted at root, applying skip rules
// at every level. Entries sorted alphabetically (dirs first then files).
func walkCanonicalTree(root string) ([]TreeNode, error) {
	return walkDir(root, "")
}

// walkDir recursively builds tree nodes for entries under absDir; relPath
// is the canonical-store-relative path-prefix for emitted nodes.
func walkDir(absDir, relPath string) ([]TreeNode, error) {
	entries, err := os.ReadDir(absDir)
	if err != nil {
		return nil, err
	}
	var nodes []TreeNode
	for _, e := range entries {
		name := e.Name()
		isDir := e.IsDir()
		// At top level (relPath == ""), apply both skip lists.
		// Nested levels skip dotfiles only — rules/projects/foo.git
		// at nested levels is gitignored via dot-prefix.
		if relPath == "" && shouldSkip(name, isDir) {
			continue
		}
		if relPath != "" && strings.HasPrefix(name, ".") {
			continue
		}
		childRel := name
		if relPath != "" {
			childRel = relPath + "/" + name
		}
		if isDir {
			children, err := walkDir(filepath.Join(absDir, name), childRel)
			if err != nil {
				return nil, err
			}
			nodes = append(nodes, TreeNode{
				Path:     childRel,
				Name:     name,
				Type:     "dir",
				Children: children,
			})
		} else {
			info, err := e.Info()
			if err != nil {
				return nil, err
			}
			nodes = append(nodes, TreeNode{
				Path:  childRel,
				Name:  name,
				Type:  "file",
				Mtime: info.ModTime().UTC().Format("2006-01-02T15:04:05Z"),
				Size:  info.Size(),
			})
		}
	}
	// Sort: dirs first, then files; alphabetical within each group.
	sort.SliceStable(nodes, func(i, j int) bool {
		if nodes[i].Type != nodes[j].Type {
			return nodes[i].Type == "dir" // dir < file
		}
		return nodes[i].Name < nodes[j].Name
	})
	return nodes, nil
}

// resolveCanonicalPath joins root + relPath safely, refusing path-escape
// attempts (..). Returns absolute path or error.
func resolveCanonicalPath(root, relPath string) (string, error) {
	// Refuse any segment that's literally ".." regardless of Clean's
	// resolution behavior. filepath.Clean would resolve "../../etc/passwd"
	// to "/etc/passwd" and strip the traversal markers, masking the
	// caller's intent. Reject the request before Clean swallows it.
	for _, seg := range strings.Split(relPath, "/") {
		if seg == ".." {
			return "", errors.New("path contains parent-traversal")
		}
	}
	clean := filepath.Clean("/" + relPath) // anchor at root
	clean = strings.TrimPrefix(clean, "/")
	if clean == "" || clean == "." {
		return "", errors.New("empty path")
	}
	// Reject dotfiles + skipped names at top level.
	parts := strings.Split(clean, "/")
	if len(parts) > 0 && parts[0] != "" {
		topName := parts[0]
		if strings.HasPrefix(topName, ".") {
			return "", errors.New("dotfile path forbidden")
		}
		isDirHint := len(parts) > 1
		if shouldSkip(topName, isDirHint) {
			return "", errors.New("path outside canonical-store class")
		}
	}
	abs := filepath.Join(root, clean)
	// Defense-in-depth: verify the joined path is still inside root.
	rel, err := filepath.Rel(root, abs)
	if err != nil || strings.HasPrefix(rel, "..") {
		return "", errors.New("path escapes canonical-store root")
	}
	return abs, nil
}

// readCanonicalFile reads the file at relPath under root, applying the
// skip-list. Returns content + mtime + error. Use for GET /api/files/{path}.
func readCanonicalFile(root, relPath string) ([]byte, string, error) {
	abs, err := resolveCanonicalPath(root, relPath)
	if err != nil {
		return nil, "", err
	}
	info, err := os.Stat(abs)
	if err != nil {
		return nil, "", err
	}
	if info.IsDir() {
		return nil, "", errors.New("path is a directory; use /api/files for tree")
	}
	data, err := os.ReadFile(abs)
	if err != nil {
		return nil, "", err
	}
	return data, info.ModTime().UTC().Format("2006-01-02T15:04:05Z"), nil
}

// statCanonical stats a file under root. Returns os.FileInfo on success.
// Helper for handlers needing mtime without read.
func statCanonical(root, relPath string) (fs.FileInfo, error) {
	abs, err := resolveCanonicalPath(root, relPath)
	if err != nil {
		return nil, err
	}
	return os.Stat(abs)
}
