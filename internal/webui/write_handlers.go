package webui

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/projects"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// cliveProposal holds a pending Clive-authored diff awaiting user approval.
// Stored in-memory keyed by proposal-id (UUID-style); 10-min TTL per
// scope-lock OQ-6 default.
type cliveProposal struct {
	id       string
	relPath  string
	content  string // proposed full content (post-diff-applied)
	purpose  string
	expiresAt time.Time
}

// proposalStore is the in-memory keyed proposal cache. Concurrency-safe.
type proposalStore struct {
	mu        sync.Mutex
	proposals map[string]*cliveProposal
}

// newProposalStore constructs an empty store.
func newProposalStore() *proposalStore {
	return &proposalStore{proposals: make(map[string]*cliveProposal)}
}

// add stores a proposal with the supplied TTL.
func (s *proposalStore) add(p *cliveProposal) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.proposals[p.id] = p
}

// get retrieves a proposal by id, returning nil if missing or expired.
// Expired proposals are evicted lazily on get.
func (s *proposalStore) get(id string) *cliveProposal {
	s.mu.Lock()
	defer s.mu.Unlock()
	p, ok := s.proposals[id]
	if !ok {
		return nil
	}
	if time.Now().After(p.expiresAt) {
		delete(s.proposals, id)
		return nil
	}
	return p
}

// remove deletes a proposal by id. Used on approve + cancel.
func (s *proposalStore) remove(id string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	delete(s.proposals, id)
}

// handleFileWrite responds to POST /api/files/{path} with mtime-check
// concurrency control per scope-lock §v3c (b). The client supplies the
// last-known mtime via the If-Match header; daemon checks against
// current file mtime + writes or returns 409 Conflict.
//
// On success (200): atomic-rename writes the new content + commits to
// the per-canonical-dir git audit + emits hub_send notification.
//
// On conflict (409): response body has `current_mtime` + `current_content`
// for client-side merge UX (overwrite/discard/merge prompt).
func (s *Server) handleFileWrite(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	relPath := strings.TrimPrefix(r.URL.Path, "/api/files/")
	if relPath == "" {
		http.Error(w, "path required", http.StatusBadRequest)
		return
	}
	// Reject revert requests routed here — they have a separate handler.
	if strings.HasSuffix(relPath, "/revert") || strings.HasSuffix(relPath, "/clive") {
		http.Error(w, "use the dedicated /clive or /revert subroute", http.StatusBadRequest)
		return
	}

	abs, err := resolveCanonicalPath(s.canonicalRoot, relPath)
	if err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}

	body, err := io.ReadAll(r.Body)
	if err != nil {
		http.Error(w, fmt.Sprintf("read body: %v", err), http.StatusBadRequest)
		return
	}

	// mtime-check (skip for new-file case — If-Match: "" means caller
	// expects the file not to exist yet).
	clientMtime := r.Header.Get("If-Match")
	info, statErr := os.Stat(abs)
	if statErr == nil {
		serverMtime := info.ModTime().UTC().Format("2006-01-02T15:04:05Z")
		if clientMtime != "" && clientMtime != serverMtime {
			currentContent, _ := os.ReadFile(abs)
			writeJSON(w, http.StatusConflict, map[string]any{
				"error":           "file changed since last read",
				"current_mtime":   serverMtime,
				"current_content": string(currentContent),
			})
			return
		}
	} else if !errors.Is(statErr, os.ErrNotExist) {
		http.Error(w, fmt.Sprintf("stat: %v", statErr), http.StatusInternalServerError)
		return
	}

	// Special-case: when path is under rules/, validate YAML against schema.
	if strings.HasPrefix(relPath, "rules/") {
		layer := layerFromRulesPath(relPath)
		v := validateRulesYAML(layer, body)
		if v.HasErrors() {
			writeJSON(w, http.StatusBadRequest, map[string]any{
				"error":    "rules validation failed",
				"errors":   v.Errors,
				"warnings": v.Warnings,
			})
			return
		}
		// Warnings allow the write; surface them in success response.
		if err := atomicWrite(abs, body); err != nil {
			http.Error(w, fmt.Sprintf("write: %v", err), http.StatusInternalServerError)
			return
		}
		newMtime, sha := s.commitAfterWrite(relPath, "user", "user@webui edit "+relPath)
		s.notifyWrite(relPath, "user", "")
		writeJSON(w, http.StatusOK, map[string]any{
			"status":   "saved",
			"mtime":    newMtime,
			"commit":   sha,
			"warnings": v.Warnings,
		})
		return
	}

	// Phase O drain #6: normalize per-project YAMLs to canonical nested
	// form on write so legacy flat-form edits persist as canonical. Read-
	// side dual-form unmarshaler handles either; write-side enforces
	// canonical to close the structural-normalization loop.
	if isProjectsYAMLPath(relPath) {
		normalized, err := projects.Normalize(body)
		if err != nil {
			http.Error(w, fmt.Sprintf("normalize: %v", err), http.StatusBadRequest)
			return
		}
		body = normalized
	}

	if err := atomicWrite(abs, body); err != nil {
		http.Error(w, fmt.Sprintf("write: %v", err), http.StatusInternalServerError)
		return
	}
	newMtime, sha := s.commitAfterWrite(relPath, "user", "user@webui edit "+relPath)
	s.notifyWrite(relPath, "user", "")
	writeJSON(w, http.StatusOK, map[string]any{
		"status": "saved",
		"mtime":  newMtime,
		"commit": sha,
	})
}

// isProjectsYAMLPath returns true for canonical-store paths under
// projects/ that end in .yaml — the scope where Phase N v3.x-2 schema-
// canonical-form (nested gates/branch/commit) applies. Other YAMLs
// (rules/general.yaml, rules/agents/*.yaml) are already nested by
// authoring convention and don't need write-side normalization.
func isProjectsYAMLPath(relPath string) bool {
	return strings.HasPrefix(relPath, "projects/") && strings.HasSuffix(relPath, ".yaml")
}

// handleCliveProposeOrApprove routes:
//   POST /api/files/{path}/clive          → propose (Clive submits diff)
//   POST /api/files/{path}/clive/approve  → user approves (apply + commit)
//   POST /api/files/{path}/clive/cancel   → user cancels
//
// MVP body for propose: {"content": "<full new content>", "purpose": "..."}.
// The diff-rendering happens client-side from the current content + the
// proposed content; server stores the proposed content for retrieval.
func (s *Server) handleCliveProposeOrApprove(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	rest := strings.TrimPrefix(r.URL.Path, "/api/files/")
	// rest is "{path}/clive" or "{path}/clive/approve" or "{path}/clive/cancel"
	var relPath, action string
	switch {
	case strings.HasSuffix(rest, "/clive"):
		relPath = strings.TrimSuffix(rest, "/clive")
		action = "propose"
	case strings.HasSuffix(rest, "/clive/approve"):
		relPath = strings.TrimSuffix(rest, "/clive/approve")
		action = "approve"
	case strings.HasSuffix(rest, "/clive/cancel"):
		relPath = strings.TrimSuffix(rest, "/clive/cancel")
		action = "cancel"
	default:
		http.NotFound(w, r)
		return
	}
	if relPath == "" {
		http.Error(w, "path required", http.StatusBadRequest)
		return
	}
	abs, err := resolveCanonicalPath(s.canonicalRoot, relPath)
	if err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}

	switch action {
	case "propose":
		var body struct {
			Content string `json:"content"`
			Purpose string `json:"purpose"`
		}
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			http.Error(w, fmt.Sprintf("decode: %v", err), http.StatusBadRequest)
			return
		}
		id := newProposalID()
		s.proposals.add(&cliveProposal{
			id:        id,
			relPath:   relPath,
			content:   body.Content,
			purpose:   body.Purpose,
			expiresAt: time.Now().Add(10 * time.Minute),
		})
		writeJSON(w, http.StatusOK, map[string]any{
			"status":      "proposed",
			"proposal_id": id,
			"path":        relPath,
		})
		// Notify user-side: SSE-Clive-activity will pick up the proposal
		// via hub-message broadcast (Clive emits hub_send when calling
		// the propose endpoint; this is in-band notification only).
	case "approve":
		var body struct {
			ProposalID string `json:"proposal_id"`
		}
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			http.Error(w, fmt.Sprintf("decode: %v", err), http.StatusBadRequest)
			return
		}
		p := s.proposals.get(body.ProposalID)
		if p == nil {
			http.Error(w, "proposal not found or expired", http.StatusNotFound)
			return
		}
		if p.relPath != relPath {
			http.Error(w, "proposal path mismatch", http.StatusBadRequest)
			return
		}
		if err := atomicWrite(abs, []byte(p.content)); err != nil {
			http.Error(w, fmt.Sprintf("write: %v", err), http.StatusInternalServerError)
			return
		}
		s.proposals.remove(body.ProposalID)
		newMtime, sha := s.commitAfterWrite(relPath, "clive", "clive@webui propose-and-apply "+relPath+" — "+p.purpose)
		s.notifyWrite(relPath, "clive", p.purpose)
		writeJSON(w, http.StatusOK, map[string]any{
			"status": "applied",
			"mtime":  newMtime,
			"commit": sha,
		})
	case "cancel":
		var body struct {
			ProposalID string `json:"proposal_id"`
		}
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			http.Error(w, fmt.Sprintf("decode: %v", err), http.StatusBadRequest)
			return
		}
		s.proposals.remove(body.ProposalID)
		writeJSON(w, http.StatusOK, map[string]any{"status": "canceled"})
	}
}

// handleFileRevert responds to POST /api/files/{path}/revert. Body:
// {"to_commit": "<sha>"}. Restores the file to the requested commit's
// content + creates a new revert-commit.
func (s *Server) handleFileRevert(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "method not allowed", http.StatusMethodNotAllowed)
		return
	}
	relPath := strings.TrimSuffix(strings.TrimPrefix(r.URL.Path, "/api/files/"), "/revert")
	if relPath == "" {
		http.Error(w, "path required", http.StatusBadRequest)
		return
	}
	if _, err := resolveCanonicalPath(s.canonicalRoot, relPath); err != nil {
		http.Error(w, err.Error(), http.StatusBadRequest)
		return
	}
	var body struct {
		ToCommit string `json:"to_commit"`
	}
	if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
		http.Error(w, fmt.Sprintf("decode: %v", err), http.StatusBadRequest)
		return
	}
	if body.ToCommit == "" {
		http.Error(w, "to_commit required", http.StatusBadRequest)
		return
	}
	if _, _, err := ensureCanonicalGit(s.canonicalRoot, relPath); err != nil {
		http.Error(w, fmt.Sprintf("ensure git: %v", err), http.StatusInternalServerError)
		return
	}
	sha, err := revertCanonicalFile(s.canonicalRoot, relPath, body.ToCommit, "user", "revert "+relPath+" → "+body.ToCommit)
	if err != nil {
		http.Error(w, fmt.Sprintf("revert: %v", err), http.StatusInternalServerError)
		return
	}
	s.notifyWrite(relPath, "user", "revert to "+body.ToCommit)
	writeJSON(w, http.StatusOK, map[string]any{
		"status": "reverted",
		"commit": sha,
	})
}

// commitAfterWrite ensures the per-dir git is initialized, then commits
// the just-written file. Returns (mtime, sha). Errors are logged and
// don't fail the user-facing write — the file is on disk regardless of
// audit-trail success per graceful-degradation lean.
func (s *Server) commitAfterWrite(relPath, author, message string) (string, string) {
	abs, _ := resolveCanonicalPath(s.canonicalRoot, relPath)
	info, _ := os.Stat(abs)
	mtime := ""
	if info != nil {
		mtime = info.ModTime().UTC().Format("2006-01-02T15:04:05Z")
	}
	if _, _, err := ensureCanonicalGit(s.canonicalRoot, relPath); err != nil {
		return mtime, "" // audit init failed; file already on disk
	}
	sha, err := commitCanonicalChange(s.canonicalRoot, relPath, author, message)
	if err != nil {
		return mtime, ""
	}
	return mtime, sha
}

// notifyWrite emits a hub_send broadcast notification (3-layer-1 visibility
// per scope-lock §v3c). Best-effort — DB errors don't fail the write.
func (s *Server) notifyWrite(relPath, actor, purpose string) {
	if s.db == nil {
		return
	}
	content := fmt.Sprintf("[webui] %s edited canonical-store path: %s", actor, relPath)
	if purpose != "" {
		content += " — " + purpose
	}
	msg := protocol.Message{
		FromAgent: "webui-daemon",
		ToAgent:   "",
		Type:      protocol.MsgUpdate,
		Content:   content,
		Created:   time.Now(),
	}
	_, _ = s.db.InsertMessage(msg)
}

// atomicWrite writes data to abs via os.WriteFile to a temp + Rename.
// Ensures partial-write isn't visible to readers.
func atomicWrite(abs string, data []byte) error {
	if err := os.MkdirAll(filepath.Dir(abs), 0o755); err != nil {
		return fmt.Errorf("mkdir parent: %w", err)
	}
	tmp := abs + ".tmp-webui"
	if err := os.WriteFile(tmp, data, 0o644); err != nil {
		return err
	}
	return os.Rename(tmp, abs)
}

// layerFromRulesPath returns the rules layer ("general" | "project" |
// "agent") for a given rules/* path. Defaults to "general" for the
// general.yaml file.
func layerFromRulesPath(relPath string) string {
	switch {
	case strings.HasPrefix(relPath, "rules/projects/"):
		return "project"
	case strings.HasPrefix(relPath, "rules/agents/"):
		return "agent"
	default:
		return "general"
	}
}

// newProposalID returns a short-ish unique identifier for proposal cache
// keys. crypto-strength not required — proposals are local-loopback only.
func newProposalID() string {
	return fmt.Sprintf("p%d-%d", time.Now().UnixNano(), os.Getpid())
}
