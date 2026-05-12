package webui

import (
	"fmt"
	"net/http"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"
)

// crossProjectCacheTTL is the lazy-compute cache lifetime per PB-2.
// Tree walks across all projects are bounded but non-trivial; 30s avoids
// repeated walks during rapid frontend refreshes while still surfacing
// new content within a UX-acceptable window.
const crossProjectCacheTTL = 30 * time.Second

// crossProjectCache holds the lazy-compute cross-project responses keyed
// by class. Mutex guards both the cache map and the time-of-entry.
type crossProjectCache struct {
	mu      sync.RWMutex
	entries map[string]crossProjectCacheEntry
}

type crossProjectCacheEntry struct {
	expires time.Time
	data    crossProjectResponse
}

// crossProjectResponse is the JSON payload returned by /api/cross-project.
type crossProjectResponse struct {
	Class    string                  `json:"class"`
	Total    int                     `json:"total"`
	Projects []crossProjectGroupItem `json:"projects"`
}

// crossProjectGroupItem groups files of one class within a single project.
// count is len(files); kept explicit so the JSON shows zero-counts for
// projects with no content in that class (per plan §2.2 Tier-1 badge UX).
type crossProjectGroupItem struct {
	Project string     `json:"project"`
	Count   int        `json:"count"`
	Files   []TreeNode `json:"files"`
}

// newCrossProjectCache returns a zero-state cache.
func newCrossProjectCache() *crossProjectCache {
	return &crossProjectCache{entries: map[string]crossProjectCacheEntry{}}
}

// lookup returns the cached entry for class if present and not expired.
func (c *crossProjectCache) lookup(class string) (crossProjectResponse, bool) {
	c.mu.RLock()
	defer c.mu.RUnlock()
	entry, ok := c.entries[class]
	if !ok || time.Now().After(entry.expires) {
		return crossProjectResponse{}, false
	}
	return entry.data, true
}

// store records a response under class with the standard TTL.
func (c *crossProjectCache) store(class string, data crossProjectResponse) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.entries[class] = crossProjectCacheEntry{
		expires: time.Now().Add(crossProjectCacheTTL),
		data:    data,
	}
}

// computeCrossProject walks every registered project's tree, gathers
// files whose Class matches the requested class string, and returns the
// grouped response. Walks via BuildFilteredTree so the HIDE → extensions
// → catch-all chain is consistent with single-project tree responses.
func computeCrossProject(canonRoot, class string) (crossProjectResponse, error) {
	if class == "" {
		return crossProjectResponse{}, fmt.Errorf("class required")
	}
	projects, err := ListProjects(canonRoot)
	if err != nil {
		return crossProjectResponse{}, err
	}
	resp := crossProjectResponse{Class: class}
	for _, p := range projects {
		rootRel := filepath.ToSlash(filepath.Join("projects", p.Name))
		tree, err := BuildFilteredTree(canonRoot, rootRel)
		if err != nil {
			continue
		}
		files := []TreeNode{}
		collectByClass(tree, class, &files)
		resp.Projects = append(resp.Projects, crossProjectGroupItem{
			Project: p.Name,
			Count:   len(files),
			Files:   files,
		})
		resp.Total += len(files)
	}
	sort.Slice(resp.Projects, func(i, j int) bool {
		return resp.Projects[i].Project < resp.Projects[j].Project
	})
	return resp, nil
}

// collectByClass recursively descends nodes appending file-typed entries
// whose Class equals the target class. Directory entries do not appear
// in the flat list (only files); their Class is only used for filter-
// gating recursion.
func collectByClass(nodes []TreeNode, class string, out *[]TreeNode) {
	for _, n := range nodes {
		if n.Type == "file" && n.Class == class {
			*out = append(*out, n)
		}
		if n.Type == "dir" && len(n.Children) > 0 {
			collectByClass(n.Children, class, out)
		}
	}
}

// handleCrossProject responds to GET /api/cross-project?class=<className>
// with files of class across all registered projects, grouped by project.
// Lazy-compute cache (30s TTL) avoids re-walks during rapid frontend
// refreshes; cache is keyed by class only (per-call scope).
func (s *Server) handleCrossProject(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	class := strings.TrimSpace(r.URL.Query().Get("class"))
	if class == "" {
		http.Error(w, "class query parameter required", http.StatusBadRequest)
		return
	}
	if s.crossProjectCache == nil {
		s.crossProjectCache = newCrossProjectCache()
	}
	if cached, ok := s.crossProjectCache.lookup(class); ok {
		writeJSON(w, http.StatusOK, cached)
		return
	}
	resp, err := computeCrossProject(s.canonicalRoot, class)
	if err != nil {
		http.Error(w, fmt.Sprintf("cross-project: %v", err), http.StatusInternalServerError)
		return
	}
	s.crossProjectCache.store(class, resp)
	writeJSON(w, http.StatusOK, resp)
}
