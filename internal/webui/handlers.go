package webui

import (
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"

	"gopkg.in/yaml.v3"
)

// handleProjects dispatches GET /api/projects → list registered projects,
// POST /api/projects → register a new project (Phase O drain per
// phase-n.md:826 register-project formal flow). bot-hq always first in
// list; others discovered from projects/*.yaml.
func (s *Server) handleProjects(w http.ResponseWriter, r *http.Request) {
	switch r.Method {
	case http.MethodGet:
		projects, err := ListProjects(s.canonicalRoot)
		if err != nil {
			http.Error(w, fmt.Sprintf("list projects: %v", err), http.StatusInternalServerError)
			return
		}
		writeJSON(w, http.StatusOK, map[string]any{"projects": projects})
	case http.MethodPost:
		s.handleProjectRegister(w, r)
	default:
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
	}
}

// projectNameRegex enforces a conservative slug format for new project
// names: lowercase ASCII alpha first, then 1-63 lowercase ASCII alpha /
// digits / hyphens. Mirrors common repo-name conventions and avoids
// filesystem collision risks (no dots, slashes, uppercase, unicode).
var projectNameRegex = regexp.MustCompile(`^[a-z][a-z0-9-]{1,63}$`)

// handleProjectRegister creates a new ~/.bot-hq/projects/<name>.yaml
// from a starter template per phase-n.md:826 register-project formal
// flow. Body shape: {"name": "<slug>", "remote_url": "<optional>"}.
// Validation: name matches projectNameRegex; "bot-hq" reserved; no
// overwrite of existing yaml. Returns 201 + {name, path} on success,
// 400 on invalid input, 409 on collision.
func (s *Server) handleProjectRegister(w http.ResponseWriter, r *http.Request) {
	var req struct {
		Name      string `json:"name"`
		RemoteURL string `json:"remote_url,omitempty"`
	}
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		http.Error(w, fmt.Sprintf("decode body: %v", err), http.StatusBadRequest)
		return
	}
	name := strings.TrimSpace(req.Name)
	if !projectNameRegex.MatchString(name) {
		http.Error(w, "name must match ^[a-z][a-z0-9-]{1,63}$", http.StatusBadRequest)
		return
	}
	if name == "bot-hq" {
		http.Error(w, "name 'bot-hq' is reserved", http.StatusBadRequest)
		return
	}
	dir := filepath.Join(s.canonicalRoot, "projects")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		http.Error(w, "mkdir failed", http.StatusInternalServerError)
		return
	}
	path := filepath.Join(dir, name+".yaml")
	body := buildProjectStarterYAML(name, strings.TrimSpace(req.RemoteURL))
	// O_CREATE|O_EXCL is atomic create-if-absent — race-free vs the
	// double-click / concurrent-POST class (Rain msg 14785 flag #2).
	f, err := os.OpenFile(path, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o644)
	if err != nil {
		if errors.Is(err, os.ErrExist) {
			http.Error(w, fmt.Sprintf("project %q already registered", name), http.StatusConflict)
			return
		}
		http.Error(w, "open failed", http.StatusInternalServerError)
		return
	}
	defer f.Close()
	if _, err := f.Write([]byte(body)); err != nil {
		http.Error(w, "write failed", http.StatusInternalServerError)
		return
	}
	writeJSON(w, http.StatusCreated, map[string]any{
		"name": name,
		"path": filepath.Join("projects", name+".yaml"),
	})
}

// buildProjectStarterYAML emits the canonical-form starter content for
// a newly-registered project. Comments cite the registration source +
// schema-canonical reference. Identity scalars (project_name +
// remote_url) at top level per Phase N v3.x-2 §2.1.
func buildProjectStarterYAML(name, remoteURL string) string {
	var b strings.Builder
	fmt.Fprintf(&b, "# %s project rules — registered via webui /api/projects\n", name)
	fmt.Fprintf(&b, "# Schema-canonical-form: nested (Phase N v3.x-2 §2.1)\n")
	fmt.Fprintf(&b, "# Edit gates / tone / greenlight / push policy below.\n")
	b.WriteString("\n")
	fmt.Fprintf(&b, "project_name: %q\n", name)
	if remoteURL != "" {
		fmt.Fprintf(&b, "remote_url: %q\n", remoteURL)
	} else {
		b.WriteString("remote_url: \"\"\n")
	}
	return b.String()
}

// handleSearch responds to GET /api/search?q=<query>&limit=N with up
// to N substring matches across canonical-store files. Phase O drain
// per phase-n.md:819 cross-search dashboard. Query must be >=2 chars
// (avoids trivial-match floods); limit defaults to 30, clamped [1,100].
func (s *Server) handleSearch(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	q := strings.TrimSpace(r.URL.Query().Get("q"))
	if len(q) < 2 {
		http.Error(w, "query must be at least 2 characters", http.StatusBadRequest)
		return
	}
	limit := 30
	if v := strings.TrimSpace(r.URL.Query().Get("limit")); v != "" {
		if n, err := strconv.Atoi(v); err == nil && n > 0 && n <= 100 {
			limit = n
		}
	}
	results, err := SearchCanonicalStore(s.canonicalRoot, q, limit)
	if err != nil {
		http.Error(w, "search failed", http.StatusInternalServerError)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"results": results})
}

// handleRecentEdits responds to GET /api/recent-edits?limit=N with the
// top-N most-recently-modified canonical-store files (mtime descending).
// limit defaults to 20, clamped [1, 100]. Phase O drain per phase-n.md
// :816 Recent-edits feed widget.
func (s *Server) handleRecentEdits(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	limit := 20
	if v := strings.TrimSpace(r.URL.Query().Get("limit")); v != "" {
		if n, err := strconv.Atoi(v); err == nil && n > 0 && n <= 100 {
			limit = n
		}
	}
	edits, err := ListRecentEdits(s.canonicalRoot, limit)
	if err != nil {
		http.Error(w, "list recent-edits failed", http.StatusInternalServerError)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"edits": edits})
}

// handleDestinations + /api/destinations route retired in Phase R3 R5 S5
// (cl-uniformity-webui-nav-refactor). The destination-allowlist nav was
// replaced by the yaml-driven tree-walker; frontend migrated to
// /api/files?tree=1 + /api/cross-project. See treewalker.go +
// crossproject.go for the replacement surface.

// handleFilesTree responds to GET /api/files with the canonical-store
// tree. Two modes per Phase R3 R5 cl-uniformity-webui-nav-refactor:
//
//   - Legacy (default, no tree=1 query): returns walkCanonicalTree result.
//     Preserves pre-Phase-R-3 behavior so existing consumers keep working
//     until S5 migrates the frontend.
//   - tree=1 mode: routes through BuildFilteredTree with the cl.IsHidden
//     filter chain + extensions allowlist classification + project_private
//     catch-all. Optional root=<canonical-rel-path> param scopes the walk
//     to a subtree (e.g., root=projects/bot-hq). Returns classified nodes
//     so the frontend can group by Class.
func (s *Server) handleFilesTree(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	if r.URL.Query().Get("tree") == "1" {
		root := r.URL.Query().Get("root")
		tree, err := BuildFilteredTree(s.canonicalRoot, root)
		if err != nil {
			http.Error(w, fmt.Sprintf("filtered walk: %v", err), http.StatusInternalServerError)
			return
		}
		writeJSON(w, http.StatusOK, map[string]any{"root": root, "tree": tree})
		return
	}
	tree, err := walkCanonicalTree(s.canonicalRoot)
	if err != nil {
		http.Error(w, fmt.Sprintf("walk: %v", err), http.StatusInternalServerError)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"tree": tree})
}

// handleExternalFile responds to GET /api/external-file/{project}/{relpath}
// — Phase Q dual-root surface for read-only reads of a registered
// project's own ~/Projects/<project>/docs/{relpath}. The project must be
// registered (have a projects/<p>.yaml) and the relpath must resolve
// strictly under the project's docs/ subdir (no traversal). Read-only —
// returns 405 on POST.
func (s *Server) handleExternalFile(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	rest := strings.TrimPrefix(r.URL.Path, "/api/external-file/")
	if rest == "" {
		http.Error(w, "project + path required", http.StatusBadRequest)
		return
	}
	parts := strings.SplitN(rest, "/", 2)
	if len(parts) < 2 || parts[0] == "" || parts[1] == "" {
		http.Error(w, "project + path required", http.StatusBadRequest)
		return
	}
	project, relpath := parts[0], parts[1]
	// Project must be registered (yaml present) — bounds the read scope
	// to known projects only.
	registered, err := ListProjects(s.canonicalRoot)
	if err != nil {
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	known := false
	for _, p := range registered {
		if p.Name == project {
			known = true
			break
		}
	}
	if !known {
		http.Error(w, "unknown project", http.StatusNotFound)
		return
	}
	home, err := os.UserHomeDir()
	if err != nil {
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	docsRoot := filepath.Join(home, "Projects", project, "docs")
	// Strict containment: relpath must not escape docsRoot via "..".
	// filepath.Rel returns a path like "../foo" or "..\foo" for traversals;
	// reject any rel starting with ".." or absolute.
	abs := filepath.Clean(filepath.Join(docsRoot, relpath))
	rel, err := filepath.Rel(docsRoot, abs)
	if err != nil || rel == ".." || strings.HasPrefix(rel, ".."+string(filepath.Separator)) || filepath.IsAbs(rel) {
		http.Error(w, "path escapes docs root", http.StatusBadRequest)
		return
	}
	// Resolve symlinks and re-check containment so a symlink under docsRoot
	// pointing outside the tree (e.g., docs/leak → /etc/passwd) can't bypass
	// the Rel guard. Both sides must resolve through the same lens — on
	// macOS, /tmp is itself a symlink to /private/tmp, so docsRoot needs
	// EvalSymlinks too or every Rel comparison would surface as escape.
	// EvalSymlinks fails on non-existent paths; surface 404 before read.
	resolved, err := filepath.EvalSymlinks(abs)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			http.NotFound(w, r)
			return
		}
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	resolvedRoot, err := filepath.EvalSymlinks(docsRoot)
	if err != nil {
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	relAfter, err := filepath.Rel(resolvedRoot, resolved)
	if err != nil || relAfter == ".." || strings.HasPrefix(relAfter, ".."+string(filepath.Separator)) || filepath.IsAbs(relAfter) {
		http.Error(w, "path escapes docs root after symlink resolution", http.StatusBadRequest)
		return
	}
	data, err := os.ReadFile(resolved)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			http.NotFound(w, r)
			return
		}
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	info, err := os.Stat(resolved)
	if err != nil {
		http.Error(w, err.Error(), http.StatusInternalServerError)
		return
	}
	mtime := info.ModTime().UTC().Format("2006-01-02T15:04:05Z")
	if r.URL.Query().Get("format") == "json" {
		writeJSON(w, http.StatusOK, map[string]any{
			"path":     "external/" + project + "/" + relpath,
			"mtime":    mtime,
			"content":  string(data),
			"external": true,
		})
		return
	}
	w.Header().Set("X-File-Mtime", mtime)
	w.Header().Set("Content-Type", "text/plain; charset=utf-8")
	_, _ = w.Write(data)
}

// handleFileContent responds to GET /api/files/{path} with file content +
// mtime. Path must resolve inside the canonical-store; dotfiles and
// skip-list entries return 404.
func (s *Server) handleFileContent(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	relPath := strings.TrimPrefix(r.URL.Path, "/api/files/")
	if relPath == "" {
		http.Error(w, "path required", http.StatusBadRequest)
		return
	}
	data, mtime, err := readCanonicalFile(s.canonicalRoot, relPath)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			http.NotFound(w, r)
			return
		}
		// Most other errors are user-input failures (bad path, etc.).
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}
	w.Header().Set("X-File-Mtime", mtime)
	if r.URL.Query().Get("format") == "json" {
		writeJSON(w, http.StatusOK, map[string]any{
			"path":    relPath,
			"mtime":   mtime,
			"content": string(data),
		})
		return
	}
	// Default content-type by extension.
	contentType := "text/plain; charset=utf-8"
	switch filepath.Ext(relPath) {
	case ".yaml", ".yml":
		contentType = "application/yaml; charset=utf-8"
	case ".json":
		contentType = "application/json; charset=utf-8"
	}
	w.Header().Set("Content-Type", contentType)
	_, _ = w.Write(data)
}

// handleRules responds to GET /api/rules with the resolved rule-set for
// optional project + agent context (deep-merge per Q-rules-3 LOCKED:
// per-project > general; per-agent applies alongside).
//
// Query params:
//
//	project=<key>  optional; merges projects/<key>.yaml on top of general
//	agent=<id>     optional; loads agents/<id>.yaml as separate "agent" key
func (s *Server) handleRules(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	project := strings.TrimSpace(r.URL.Query().Get("project"))
	agent := strings.TrimSpace(r.URL.Query().Get("agent"))

	resolved, err := resolveRules(s.canonicalRoot, project, agent)
	if err != nil {
		http.Error(w, fmt.Sprintf("resolve rules: %v", err), http.StatusInternalServerError)
		return
	}
	writeJSON(w, http.StatusOK, resolved)
}

// resolveRules deep-merges general.yaml with projects/<project>.yaml (if
// project provided) and adds agents/<agent>.yaml as a separate "agent"
// key (if agent provided). Missing files are silently treated as empty —
// substrate can predate adoption per Phase O migration plan.
func resolveRules(root, project, agent string) (map[string]any, error) {
	merged := map[string]any{}
	// Layer 1: general
	if data, err := os.ReadFile(filepath.Join(root, "rules", "general.yaml")); err == nil {
		var m map[string]any
		if err := yaml.Unmarshal(data, &m); err != nil {
			return nil, fmt.Errorf("general.yaml: %w", err)
		}
		merged = m
	} else if !errors.Is(err, os.ErrNotExist) {
		return nil, err
	}
	// Layer 2: project (deep-merge over general)
	if project != "" {
		path := filepath.Join(root, "rules", "projects", project+".yaml")
		if data, err := os.ReadFile(path); err == nil {
			var pm map[string]any
			if err := yaml.Unmarshal(data, &pm); err != nil {
				return nil, fmt.Errorf("projects/%s.yaml: %w", project, err)
			}
			merged = deepMerge(merged, pm)
		} else if !errors.Is(err, os.ErrNotExist) {
			return nil, err
		}
	}
	// Layer 3: agent (separate key, not merged into top-level)
	if agent != "" {
		path := filepath.Join(root, "rules", "agents", agent+".yaml")
		if data, err := os.ReadFile(path); err == nil {
			var am map[string]any
			if err := yaml.Unmarshal(data, &am); err != nil {
				return nil, fmt.Errorf("agents/%s.yaml: %w", agent, err)
			}
			merged["agent"] = am
		} else if !errors.Is(err, os.ErrNotExist) {
			return nil, err
		}
	}
	return merged, nil
}

// deepMerge recursively merges over into base; over keys win on conflict.
// Maps merge recursively; non-map values overwrite.
func deepMerge(base, over map[string]any) map[string]any {
	out := make(map[string]any, len(base)+len(over))
	for k, v := range base {
		out[k] = v
	}
	for k, ov := range over {
		if bv, has := out[k]; has {
			bm, bok := bv.(map[string]any)
			om, ook := ov.(map[string]any)
			if bok && ook {
				out[k] = deepMerge(bm, om)
				continue
			}
		}
		out[k] = ov
	}
	return out
}

// handleSessions responds to GET /api/sessions with the parsed sessions
// rolling index from ~/.bot-hq/sessions/index.md. Filter by project query
// param.
//
// MVP returns the raw index content; later versions parse to structured
// JSON. Sessions are a thin slice of canonical-store; the rich view is
// served from the manifests at sessions/<id>/manifest.md (accessed via
// the file-content endpoint with the explicit path).
func (s *Server) handleSessions(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	indexPath := filepath.Join(s.canonicalRoot, "sessions", "index.md")
	data, err := os.ReadFile(indexPath)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			// No sessions yet — return empty payload.
			writeJSON(w, http.StatusOK, map[string]any{"index": ""})
			return
		}
		http.Error(w, fmt.Sprintf("read sessions index: %v", err), http.StatusInternalServerError)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"index": string(data)})
}

// handleCliveActivity responds to GET /api/clive/activity. When the
// request carries Accept: text/event-stream, dispatches to the SSE
// live-feed branch (P-1, sse.go); otherwise returns the JSON snapshot
// of the last N=50 clive-authored hub messages.
//
// Returns 503 when hub.DB unavailable (CI / test config) so the frontend
// can render a graceful "no Clive activity" placeholder.
func (s *Server) handleCliveActivity(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	if strings.Contains(r.Header.Get("Accept"), "text/event-stream") {
		s.handleCliveActivitySSE(w, r)
		return
	}
	if s.db == nil {
		writeJSON(w, http.StatusServiceUnavailable, map[string]any{"error": "hub.DB not configured"})
		return
	}
	msgs, err := s.db.ReadMessages("clive", 0, 50)
	if err != nil {
		http.Error(w, fmt.Sprintf("read clive activity: %v", err), http.StatusInternalServerError)
		return
	}
	// Pre-filter: only messages from clive (read API returns to+from);
	// downstream UI shows from-clive only.
	var out []any
	for _, m := range msgs {
		if m.FromAgent == "clive" {
			out = append(out, map[string]any{
				"id":         m.ID,
				"to_agent":   m.ToAgent,
				"type":       string(m.Type),
				"content":    m.Content,
				"created":    m.Created.UTC().Format("2006-01-02T15:04:05Z"),
				"session_id": m.SessionID,
			})
		}
	}
	writeJSON(w, http.StatusOK, map[string]any{"messages": out})
}

// handleFileHistory responds to GET /api/files/{path}/history with the
// per-dir-git commit history for the file. Empty list when the file's
// top-dir has no .git/ initialized yet (graceful — frontend renders a
// "no history" placeholder rather than erroring).
//
// Query params: ?limit=<n> caps the returned commits (default 50, hard
// max 200 to bound response size + git-log time).
func (s *Server) handleFileHistory(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	relPath := strings.TrimSuffix(strings.TrimPrefix(r.URL.Path, "/api/files/"), "/history")
	if relPath == "" {
		http.Error(w, "path required", http.StatusBadRequest)
		return
	}
	if _, err := resolveCanonicalPath(s.canonicalRoot, relPath); err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}
	limit := 50
	if raw := r.URL.Query().Get("limit"); raw != "" {
		var n int
		if _, err := fmt.Sscanf(raw, "%d", &n); err == nil && n > 0 {
			limit = n
		}
	}
	if limit > 200 {
		limit = 200
	}
	commits, err := fileHistory(s.canonicalRoot, relPath, limit)
	if err != nil {
		http.Error(w, fmt.Sprintf("history: %v", err), http.StatusInternalServerError)
		return
	}
	if commits == nil {
		commits = []CommitInfo{} // ensure JSON [] not null
	}
	writeJSON(w, http.StatusOK, map[string]any{"commits": commits})
}

// writeJSON encodes v to w with the given status code + JSON content type.
// Best-effort — error in encoding logs but doesn't double-write headers.
func writeJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.WriteHeader(status)
	enc := json.NewEncoder(w)
	enc.SetIndent("", "  ")
	_ = enc.Encode(v)
}
