package mcp

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/google/uuid"
	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
	"github.com/gregoryerrl/bot-hq/internal/sessions"
	"github.com/mark3labs/mcp-go/mcp"
)

// splitComma splits a comma-separated string into a trimmed, non-empty slice.
func splitComma(s string) []string {
	if s == "" {
		return nil
	}
	parts := strings.Split(s, ",")
	out := make([]string, 0, len(parts))
	for _, p := range parts {
		p = strings.TrimSpace(p)
		if p != "" {
			out = append(out, p)
		}
	}
	return out
}

// hubSessionClose stores the calling agent's final SNAP into session_ledger.
// Voluntary-only by design (Phase H slice 4 C4 / H-15) — agents invoke this
// before graceful idle so the next register call surfaces it as
// `last_session_snap` for cold-start context. Best-effort: rebuild-mid-flight
// or hard-crash close never fires, which is acceptable per the H-27
// deferral rationale.
func hubSessionClose(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_session_close",
		mcp.WithDescription("Voluntarily store this agent's final SNAP into the session ledger before graceful idle. The next hub_register for this agent_id will return it as last_session_snap for cold-start bootstrap context. Best-effort: rebuild-mid-flight or crash-close skips it."),
		mcp.WithString("agent_id", mcp.Required(), mcp.Description("Agent ID whose SNAP is being stored")),
		mcp.WithString("snap_text", mcp.Required(), mcp.Description("Final SNAP text (typically the SNAP-block content from the agent's last cycle)")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		agentID, err := req.RequireString("agent_id")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		if strings.TrimSpace(agentID) == "" {
			return mcp.NewToolResultError("agent_id must not be empty"), nil
		}
		snapText, err := req.RequireString("snap_text")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		if err := db.StoreSessionClose(agentID, snapText); err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("session_close failed: %v", err)), nil
		}

		return mcp.NewToolResultText(toJSON(map[string]string{
			"status":   "stored",
			"agent_id": agentID,
		})), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

func hubSessionCreate(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_session_create",
		mcp.WithDescription("Create a new session in the hub"),
		mcp.WithString("mode", mcp.Required(), mcp.Description("Session mode: brainstorm, implement, chat")),
		mcp.WithString("purpose", mcp.Required(), mcp.Description("What the session is for")),
		mcp.WithString("agents", mcp.Description("Comma-separated agent IDs to include")),
		mcp.WithString("project", mcp.Description("Project key (e.g., bot-hq, bcc-ad-manager). When set, writes a session-cluster manifest at ~/.bot-hq/sessions/<YYYY-MM-DD-project>/manifest.md alongside the in-db session record.")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		mode, err := req.RequireString("mode")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		purpose, err := req.RequireString("purpose")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		sm := protocol.SessionMode(mode)
		if !sm.Valid() {
			return mcp.NewToolResultError(fmt.Sprintf("invalid session mode: %s", mode)), nil
		}

		agentList := splitComma(req.GetString("agents", ""))
		sess := protocol.Session{
			ID:      uuid.New().String(),
			Mode:    sm,
			Purpose: purpose,
			Agents:  agentList,
			Status:  protocol.SessionActive,
			Created: time.Now(),
		}

		if err := db.CreateSession(sess); err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("create session failed: %v", err)), nil
		}

		// Phase N v3 v3a writer-flow wiring: when project is supplied,
		// also write a session-cluster manifest at SessionsDir()/<id>/
		// manifest.md alongside the in-db session record. The session-
		// cluster id format is YYYY-MM-DD-<project> per Q-I RATIFIED;
		// distinct from the in-db UUID. WriteManifest is idempotent so
		// repeat calls within the same UTC day no-op or refresh fields.
		// WriteIndex rebuilds the rolling index post-write.
		response := map[string]string{
			"status":     "created",
			"session_id": sess.ID,
		}
		if project := strings.TrimSpace(req.GetString("project", "")); project != "" {
			// Phase W pivot enforcement: detect active session for any
			// other project and reject. Forces explicit hub_session_finalize
			// before pivoting — preserves close-summary discipline.
			if otherID, otherProject, ferr := sessions.FindActiveForAnyOtherProject(project); ferr == nil && otherID != "" {
				return mcp.NewToolResultError(fmt.Sprintf(
					"active session %q exists for project %q — call hub_session_finalize with project=%q + outcome before pivoting to %q",
					otherID, otherProject, otherProject, project,
				)), nil
			}

			// Phase W multi-session-per-day support: NextAvailableSessionID
			// returns "YYYY-MM-DD-<project>" for the first session of the
			// day (back-compat) and -2 / -3 / etc. for subsequent same-day
			// sessions. Agent calls hub_session_create per work scope.
			clusterID, idErr := sessions.NextAvailableSessionID(time.Now(), project)
			if idErr != nil {
				response["manifest_write_error"] = fmt.Sprintf("next-id resolution: %v", idErr)
			} else {
				// StartMsgID seeds from the latest hub msg-id at create
				// time so Finalize can compute MsgCount = end - start.
				var startMsgID int
				if recent, rerr := db.GetRecentMessages(1); rerr == nil && len(recent) > 0 {
					startMsgID = int(recent[0].ID)
				}

				manifest := sessions.Manifest{
					ID:         clusterID,
					Project:    project,
					StartTS:    sess.Created,
					StartMsgID: startMsgID,
					Agents:     agentList,
					Status:     "active",
					Body:       fmt.Sprintf("Session purpose: %s\nMode: %s\nIn-db session UUID: %s\n", purpose, mode, sess.ID),
				}
				if werr := sessions.WriteManifest(manifest); werr != nil {
					// Don't fail the in-db session create on manifest-write
					// error — log via response and let caller decide. Per
					// Phase N v2 §3 graceful-degradation lean.
					response["manifest_write_error"] = werr.Error()
				} else {
					response["session_cluster_id"] = clusterID
					if ierr := sessions.WriteIndex(); ierr != nil {
						response["index_write_error"] = ierr.Error()
					}
				}
			}
		}

		return mcp.NewToolResultText(toJSON(response)), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

// hubSessionLoad implements the Phase N v2 #5 N-1(b)-B MCP tool that
// reads the session-cluster manifest at ~/.bot-hq/sessions/<id>/
// manifest.md and returns its raw content + path. Per N-1 (a) Q-IV
// RATIFIED lean (iii) CLI + file: this is the MCP-side surface paired
// with the bot-hq session-load CLI subcommand.
//
// Optional `project` param: when supplied (and `session_id` omitted),
// returns the most-recent session-id matching that project key per
// Q-V RATIFIED auto-load-most-recent semantics.
func hubSessionLoad() ToolDef {
	tool := mcp.NewTool("hub_session_load",
		mcp.WithDescription("Load a session-cluster manifest by ID, or look up the most-recent session for a project. Returns the manifest.md content + path."),
		mcp.WithString("session_id", mcp.Description("Session ID (e.g., 2026-05-05-bot-hq). When omitted, project must be provided.")),
		mcp.WithString("project", mcp.Description("Project key (e.g., bot-hq). When session_id is omitted, returns the most-recent session for this project.")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		id := req.GetString("session_id", "")
		project := req.GetString("project", "")
		if id == "" && project == "" {
			return mcp.NewToolResultError("hub_session_load: session_id or project required"), nil
		}
		if id == "" {
			recent, err := sessions.MostRecentForProject(project)
			if err != nil {
				return mcp.NewToolResultError(fmt.Sprintf("most-recent lookup failed: %v", err)), nil
			}
			if recent == "" {
				return mcp.NewToolResultError(fmt.Sprintf("no sessions found for project %q", project)), nil
			}
			id = recent
		}
		content, err := sessions.LoadManifestContent(id)
		if err != nil {
			if os.IsNotExist(err) {
				return mcp.NewToolResultError(fmt.Sprintf("session manifest not found: %s", id)), nil
			}
			return mcp.NewToolResultError(fmt.Sprintf("load manifest failed: %v", err)), nil
		}
		return mcp.NewToolResultText(toJSON(map[string]string{
			"session_id": id,
			"path":       sessions.ManifestPath(id),
			"content":    content,
		})), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

// hubSessionCheckpoint implements Phase R R5 (d-2) checkpoint MCP tool.
// Writes structured state (active_workstream / last_commit_sha / phase /
// posture) to the session manifest at fire-time, refreshes
// checkpoint_ts, and optionally appends a free-form body chunk.
//
// Idempotent: empty fields skip-update; only non-empty fields overwrite.
// Returns os.ErrNotExist if the session_id has no manifest yet (caller
// must hub_session_create first).
//
// Per phase-r.md R5 cluster + Rain msg 15545 BRAIN-2nd Refine-B
// backwards-compat shim. Ratchet: substring locks on tool name + each
// optional field name.
func hubSessionCheckpoint() ToolDef {
	tool := mcp.NewTool("hub_session_checkpoint",
		mcp.WithDescription("Write structured checkpoint state into a session manifest. Idempotent — empty fields preserve existing values; non-empty fields overwrite. Use at boundary moments (phase-open / commit-land / pivot / etc.) to capture active workstream + last commit + phase + posture."),
		mcp.WithString("session_id", mcp.Required(), mcp.Description("Session-cluster ID (e.g., 2026-05-08-bot-hq)")),
		mcp.WithString("active_workstream", mcp.Description("Brief label of the in-flight work-cluster (e.g., 'Phase R R5 sessions')")),
		mcp.WithString("last_commit_sha", mcp.Description("HEAD SHA at checkpoint time")),
		mcp.WithString("phase", mcp.Description("Active phase identifier (e.g., 'Phase-R-OPEN')")),
		mcp.WithString("posture", mcp.Description("EYES/HANDS/BRAIN-class posture")),
		mcp.WithString("body_append", mcp.Description("Optional free-form markdown chunk appended to manifest body with timestamp header")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		id, err := req.RequireString("session_id")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		cp := sessions.CheckpointFields{
			ActiveWorkstream: req.GetString("active_workstream", ""),
			LastCommitSHA:    req.GetString("last_commit_sha", ""),
			Phase:            req.GetString("phase", ""),
			Posture:          req.GetString("posture", ""),
			BodyAppend:       req.GetString("body_append", ""),
		}
		if err := sessions.WriteCheckpoint(id, cp); err != nil {
			if os.IsNotExist(err) {
				return mcp.NewToolResultError(fmt.Sprintf("session manifest not found: %s — call hub_session_create first", id)), nil
			}
			return mcp.NewToolResultError(fmt.Sprintf("write checkpoint failed: %v", err)), nil
		}
		return mcp.NewToolResultText(toJSON(map[string]string{
			"status":       "checkpointed",
			"session_id":   id,
			"manifest_path": sessions.ManifestPath(id),
		})), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

// hubSessionArchive implements Phase R R5 (d-3) cite-anchor-preserving
// retention archive. Wraps sessions.PruneOlderThanWithCitePreservation
// — purges session manifests older than retention_days but preserves
// any session whose msg-id range is cited in the canonical 7-path-class
// scan-set (discipline-log.md / phase/*.md / docs/arcs/*.md / ratchets/
// active.md + closed snapshots / projects/<p>/*.md / per-agent/
// discipline-anchors.md).
//
// Optional dry_run param: returns the would-prune + would-exempt lists
// without deleting anything. Default retention_days = 30 per
// sessions.DefaultRetentionDays.
//
// Per phase-r.md R5 cluster + Rain msg 15545 BRAIN-2nd Refine-C
// (7-path-class scan + cache + mtime-invalidation lean) + msg 15553
// (dry-run flag + tool-surface-schema BRAIN-2nd).
func hubSessionArchive() ToolDef {
	tool := mcp.NewTool("hub_session_archive",
		mcp.WithDescription("Archive (delete) session manifests older than retention_days, exempting any session whose msg-id range is cited in canonical-store cite-anchor docs (discipline-log / phase / arcs / ratchets / projects / per-agent discipline-anchors). Default retention 30 days. Set dry_run=true to preview without deleting."),
		mcp.WithNumber("retention_days", mcp.Description("Retention window in days. Default: 30. <=0 disables (no-op).")),
		mcp.WithBoolean("dry_run", mcp.Description("When true, return would-prune + would-exempt lists without deleting. Default false.")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		retentionDays := int(req.GetFloat("retention_days", float64(sessions.DefaultRetentionDays)))
		dryRun := req.GetBool("dry_run", false)

		home, _ := os.UserHomeDir()
		canonRoot := filepath.Join(home, ".bot-hq")
		repoRoot := filepath.Join(home, "Projects", "bot-hq")

		if dryRun {
			cited, err := sessions.ScanCitedMsgIDs(canonRoot, repoRoot)
			if err != nil {
				return mcp.NewToolResultError(fmt.Sprintf("scan cited msg-ids: %v", err)), nil
			}
			ids, err := sessions.ListSessionIDs()
			if err != nil {
				return mcp.NewToolResultError(fmt.Sprintf("list sessions: %v", err)), nil
			}
			cutoff := time.Now().Add(-time.Duration(retentionDays) * 24 * time.Hour)
			var wouldPrune, wouldExempt []string
			for _, id := range ids {
				m, err := sessions.ReadManifest(id)
				if err != nil {
					continue
				}
				age := pickAgePublic(m)
				if age.IsZero() || !age.Before(cutoff) {
					continue
				}
				if sessions.SessionRangeOverlapsCited(m, cited) {
					wouldExempt = append(wouldExempt, id)
				} else {
					wouldPrune = append(wouldPrune, id)
				}
			}
			result := map[string]any{
				"status":          "dry_run",
				"retention_days":  retentionDays,
				"cited_msg_count": len(cited),
				"would_prune":     wouldPrune,
				"would_exempt":    wouldExempt,
			}
			return mcp.NewToolResultText(toJSON(result)), nil
		}

		pruned, exempted, err := sessions.PruneOlderThanWithCitePreservation(retentionDays, time.Now(), canonRoot, repoRoot)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("archive failed: %v", err)), nil
		}
		result := map[string]any{
			"status":         "archived",
			"retention_days": retentionDays,
			"pruned":         pruned,
			"exempted":       exempted,
		}
		return mcp.NewToolResultText(toJSON(result)), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

// pickAgePublic mirrors sessions.pickAge for dry-run path (the
// non-exported helper isn't reachable from this package). Returns the
// most-recent timestamp from a manifest for retention-window comparison.
func pickAgePublic(m sessions.Manifest) time.Time {
	if !m.EndTS.IsZero() {
		return m.EndTS
	}
	return m.StartTS
}

func hubSessionJoin(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_session_join",
		mcp.WithDescription("Join an existing session"),
		mcp.WithString("session_id", mcp.Required(), mcp.Description("Session ID to join")),
		mcp.WithString("agent_id", mcp.Required(), mcp.Description("Agent ID joining the session")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		sessionID, err := req.RequireString("session_id")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		agentID, err := req.RequireString("agent_id")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		if err := db.JoinSession(sessionID, agentID); err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("join session failed: %v", err)), nil
		}

		return mcp.NewToolResultText(toJSON(map[string]string{
			"status":     "joined",
			"session_id": sessionID,
			"agent_id":   agentID,
		})), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

// hubSessionFinalize implements Phase W close-summary writing for the
// session-cluster manifest. Distinct from hub_session_close (which
// stores per-agent SNAP into hub.DB for cold-start bootstrap context).
//
// Behavior:
//  1. FindActiveForProject(project) — locates the active manifest.
//  2. ReadMessages(agentID="", sinceID=StartMsgID, limit=10000) — pulls
//     the hub message window for decision extraction.
//  3. GetRecentMessages(1) — gets the latest msg-id for MsgCount cache.
//  4. ExtractGitChanges(repo_path) — populates CommitsLanded +
//     FilesTouched (skipped when repo_path is empty).
//  5. sessions.Finalize(...) — writes the rich-payload manifest +
//     rebuilds the index.
func hubSessionFinalize(db *hub.DB) ToolDef {
	tool := mcp.NewTool("hub_session_finalize",
		mcp.WithDescription("Close the active session-cluster manifest for a project with rich retrospective payload (outcome narrative + auto-extracted CommitsLanded / FilesTouched / Decisions / MsgCount). Phase W sessions hardening."),
		mcp.WithString("project", mcp.Required(), mcp.Description("Project key whose active session is being finalized (e.g., bot-hq, bcc-ad-manager)")),
		mcp.WithString("outcome", mcp.Required(), mcp.Description("Agent-authored narrative summarizing what happened in this session. Multi-paragraph fine. Required for retrospective utility.")),
		mcp.WithString("status", mcp.Description("Session close status: closed (default) | closed-pivoted-out | closed-eod | closed-auto-migrated. Determines retrospective category.")),
		mcp.WithString("repo_path", mcp.Description("Optional git repo path for CommitsLanded + FilesTouched extraction (e.g., /Users/gregoryerrl/Projects/bot-hq). Skipped when omitted.")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		project, err := req.RequireString("project")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		outcome, err := req.RequireString("outcome")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		status := req.GetString("status", "closed")
		repoPath := req.GetString("repo_path", "")

		activeID, err := sessions.FindActiveForProject(project)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("find active for %s: %v", project, err)), nil
		}
		if activeID == "" {
			return mcp.NewToolResultError(fmt.Sprintf("no active session found for project %q", project)), nil
		}

		manifest, err := sessions.ReadManifest(activeID)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("read active manifest: %v", err)), nil
		}

		// Pull hub messages between [StartMsgID, latest] for decision extraction.
		// Empty agentID = no per-agent filter. Limit=10000 covers a generous
		// session window without needing pagination for typical sessions.
		var msgs []protocol.Message
		if manifest.StartMsgID > 0 {
			msgs, err = db.ReadMessages("", int64(manifest.StartMsgID), 10000)
			if err != nil {
				return mcp.NewToolResultError(fmt.Sprintf("read messages: %v", err)), nil
			}
		}

		// Latest msg-id for MsgCount cache + EndMsgID. GetRecentMessages(1)
		// returns the most-recent row regardless of agent.
		var latestMsgID int
		recent, rerr := db.GetRecentMessages(1)
		if rerr == nil && len(recent) > 0 {
			latestMsgID = int(recent[0].ID)
		}

		closed, err := sessions.Finalize(activeID, sessions.FinalizeOptions{
			Outcome:     outcome,
			Status:      status,
			Now:         time.Now().UTC(),
			Messages:    msgs,
			RepoPath:    repoPath,
			LatestMsgID: latestMsgID,
		})
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("finalize: %v", err)), nil
		}

		return mcp.NewToolResultText(toJSON(map[string]any{
			"status":          "finalized",
			"session_id":      closed.ID,
			"project":         closed.Project,
			"end_ts":          closed.EndTS.Format(time.RFC3339),
			"msg_count":       closed.MsgCount,
			"decisions_count": len(closed.Decisions),
			"commits_count":   len(closed.CommitsLanded),
			"files_count":     len(closed.FilesTouched),
		})), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

// hubSessionLookback returns a markdown retrospective rendering of a
// session-cluster manifest. Audience is the trio (brian/rain/emma) —
// optimized format leads with outcome narrative + structured fields
// so retro is fast to scan.
//
// Phase W sessions hardening 2026-05-10.
func hubSessionLookback() ToolDef {
	tool := mcp.NewTool("hub_session_lookback",
		mcp.WithDescription("Retrospective markdown for a session-cluster manifest. Optimized for trio consumption — outcome narrative + structured fields (decisions, commits, files) + raw body. Use to scan a single past session for patterns, decisions, friction points."),
		mcp.WithString("session_id", mcp.Required(), mcp.Description("Session cluster ID (e.g., 2026-05-10-bot-hq, 2026-05-10-bcc-ad-manager-2)")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		sessionID, err := req.RequireString("session_id")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		md, err := sessions.Lookback(sessionID)
		if err != nil {
			if sessions.LookbackErrIsMissing(err) {
				return mcp.NewToolResultError(fmt.Sprintf("session not found: %s", sessionID)), nil
			}
			return mcp.NewToolResultError(fmt.Sprintf("lookback %s: %v", sessionID, err)), nil
		}
		return mcp.NewToolResultText(md), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}
