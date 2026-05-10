package webui

import (
	"errors"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

// TreeNode is a single entry in the canonical-store tree response. Either
// File (with Mtime + Size) or Dir (with Children). For destination
// resolvers, Missing=true marks a blank-state placeholder (e.g., a
// project's overview.md not yet authored).
type TreeNode struct {
	Path     string     `json:"path"` // canonical-store-relative; "external/<project>/<rel>" for dual-root project-docs entries
	Name     string     `json:"name"` // basename
	Type     string     `json:"type"` // "file" or "dir"
	Mtime    string     `json:"mtime,omitempty"`
	Size     int64      `json:"size,omitempty"`
	Children []TreeNode `json:"children,omitempty"`
	Missing  bool       `json:"missing,omitempty"`
	// External marks a Phase Q dual-root entry — the file lives outside
	// the canonical store (under ~/Projects/<project>/docs/) and is
	// surfaced read-only via /api/external-file. Frontend disables save
	// + revert affordances when External=true.
	External bool `json:"external,omitempty"`
}

// canonicalSkipList is the set of top-level entries excluded from the
// legacy tree-walk endpoint. Phase N v3.x-1 retires the tree-walk in
// favor of destination-allowlist nav (destinations.go); the list is kept
// for backward-compatible /api/files behaviour during the v3.x-1 cutover.
//
//   - hub.db / webui-index.db — sqlite runtime
//   - live.log / debug.log — runtime logs
//   - gates/ — surfaced via per-project Rules destination, not raw walk
//   - sessions/ — surfaced via /api/sessions + Sessions destination
var canonicalSkipList = map[string]struct{}{
	"hub.db":         {},
	"hub.db-shm":     {},
	"hub.db-wal":     {},
	"bot-hq.db":      {},
	"live.log":       {},
	"debug.log":      {},
	"gates":          {},
	"sessions":       {},
	"webui-index.db": {},
}

// canonicalSkipAgentDirs lists per-agent dirs whose contents are mostly
// runtime (last_state.json) but contain selected allowlisted files
// (discipline-anchors.md). The walk-skip leaves it to per-destination
// resolvers to surface allowlisted paths; resolveCanonicalPath whitelists
// specific subpaths (see allowlistedAgentSubpaths).
var canonicalSkipAgentDirs = map[string]struct{}{
	"brian": {},
	"rain":  {},
	"emma":  {},
	"clive": {},
}

// allowlistedAgentSubpaths lists the only files under <agent>/ that the
// file-content endpoint will serve. Per scope-lock-v4.2 HIDE list:
// last_state.json (per-agent runtime) is hidden; discipline-anchors.md
// (Agent Notes destination) is allowed.
var allowlistedAgentSubpaths = map[string]struct{}{
	"discipline-anchors.md": {},
}

// hideListExtensions are file extensions whose content-class is HIDE per
// scope-lock-v4.2 (binary / runtime / log / source-code).
var hideListExtensions = map[string]struct{}{
	".db":    {},
	".log":   {},
	".jsonl": {},
	".json":  {}, // runtime JSON; allowlist exceptions go via specific paths
	".ts":    {}, // plugins/github source
	".js":    {}, // plugins/github source
}

// hideListBasenames are specific filenames that fail S3 (audit/hook trail).
var hideListBasenames = map[string]struct{}{
	"voice-mirror-log.md": {},
	"last_state.json":     {},
}

// hideListTopDirs are top-level directories whose entire contents are HIDE
// (non-md/yaml runtime + source). Note: contents may contain md files
// (e.g., plugins/github/README.md) but per scope-lock §3.2 these are
// internal-code-class and excluded.
var hideListTopDirs = map[string]struct{}{
	"diag":      {},
	"sentinels": {},
	"bridge":    {},
	"plugins":   {}, // plugins/github source
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
		if _, ok := hideListTopDirs[name]; ok {
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

// RecentEdit is a flattened view of a single canonical-store file
// suitable for the recent-edits feed widget (Phase O drain per
// phase-n.md:816). Same fields as the file-typed TreeNode but flat
// (no nesting; path includes the directory prefix).
type RecentEdit struct {
	Path  string `json:"path"`
	Name  string `json:"name"`
	Mtime string `json:"mtime"`
	Size  int64  `json:"size"`
}

// ListRecentEdits walks the canonical-store tree, flattens to file
// entries only, sorts by mtime descending (most-recent first), and
// returns the top N. Reuses walkCanonicalTree skip-list discipline so
// runtime-state files (hub.db, agent dirs, .git, etc.) never surface.
// limit is clamped to [1, 100] by the caller; 0 or negative → 20 default
// is the caller's responsibility (handler enforces).
func ListRecentEdits(root string, limit int) ([]RecentEdit, error) {
	tree, err := walkCanonicalTree(root)
	if err != nil {
		return nil, err
	}
	var flat []RecentEdit
	flattenForRecent(tree, &flat)
	sort.SliceStable(flat, func(i, j int) bool {
		return flat[i].Mtime > flat[j].Mtime
	})
	if len(flat) > limit {
		flat = flat[:limit]
	}
	return flat, nil
}

// flattenForRecent depth-first appends file entries to out, descending
// into dir children. dir nodes themselves are not added (only files).
func flattenForRecent(nodes []TreeNode, out *[]RecentEdit) {
	for _, n := range nodes {
		switch n.Type {
		case "file":
			*out = append(*out, RecentEdit{
				Path:  n.Path,
				Name:  n.Name,
				Mtime: n.Mtime,
				Size:  n.Size,
			})
		case "dir":
			flattenForRecent(n.Children, out)
		}
	}
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

		// Allowlist exceptions for paths that shouldSkip would reject but
		// destinations legitimately surface:
		//   - sessions/<id>/manifest.md (Sessions destination)
		//   - gates/*.md (per-project Rules for bot-hq)
		//   - <agent>/discipline-anchors.md (Agent Notes destination)
		_, isAgentDir := canonicalSkipAgentDirs[topName]
		_, isAllowedAgentSub := allowlistedAgentSubpaths[parts[len(parts)-1]]
		isAllowedAgentPath := isAgentDir && len(parts) >= 2 && isAllowedAgentSub
		isAllowedSession := topName == "sessions" && len(parts) >= 3 && parts[len(parts)-1] == "manifest.md"
		isAllowedGate := topName == "gates" && len(parts) == 2 && strings.HasSuffix(parts[1], ".md")

		if !isAllowedAgentPath && !isAllowedSession && !isAllowedGate {
			if shouldSkip(topName, isDirHint) {
				return "", errors.New("path outside canonical-store class")
			}
		}
	}

	// Apply HIDE-list filters by basename + extension regardless of dir.
	base := filepath.Base(clean)
	if _, ok := hideListBasenames[base]; ok {
		return "", errors.New("path is HIDE-list basename")
	}
	ext := filepath.Ext(base)
	if _, ok := hideListExtensions[ext]; ok {
		return "", errors.New("path has HIDE-list extension")
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

