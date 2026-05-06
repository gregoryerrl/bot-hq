package webui

import (
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"strings"

	"gopkg.in/yaml.v3"
)

// handleFilesTree responds to GET /api/files with the canonical-store
// tree (excluding runtime state per shouldSkip rules).
func (s *Server) handleFilesTree(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	tree, err := walkCanonicalTree(s.canonicalRoot)
	if err != nil {
		http.Error(w, fmt.Sprintf("walk: %v", err), http.StatusInternalServerError)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"tree": tree})
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

// handleCliveActivity responds to GET /api/clive/activity with recent
// hub messages from agent_id=clive. MVP returns a snapshot (last N=50);
// SSE streaming wired in v3c per OQ-2 LOCKED.
//
// Returns 503 when hub.DB unavailable (CI / test config) so the frontend
// can render a graceful "no Clive activity" placeholder.
func (s *Server) handleCliveActivity(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodGet {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
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

// writeJSON encodes v to w with the given status code + JSON content type.
// Best-effort — error in encoding logs but doesn't double-write headers.
func writeJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.WriteHeader(status)
	enc := json.NewEncoder(w)
	enc.SetIndent("", "  ")
	_ = enc.Encode(v)
}
