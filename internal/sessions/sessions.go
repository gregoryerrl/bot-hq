// Package sessions implements the Phase N v2 #4 N-1(b)-A boundary-
// detector + manifest-author per N-1 (a) RATIFIED design-spike at
// docs/plans/2026-05-05-phase-n-N-1-id-based-sessions-design-spike.md.
//
// Scope (this commit):
//   - MakeSessionID — canonical YYYY-MM-DD-<project> id format (per Q-I)
//   - DetectBoundaryFromUserMsg — T-3 explicit-phrase + T-6 rebuild-
//     restart triggers (pure user-msg-only triggers per OQ-1a RATIFIED)
//   - Manifest type + WriteManifest + ReadManifest — frontmatter strict +
//     body free-form (per Q-II) at ~/.bot-hq/sessions/<id>/manifest.md
//
// Deferred to follow-up commits in Phase N v2:
//   - T-1 (project pivot via cwd/cite-change) — needs runtime context
//   - T-2 (time gap >8h) — needs hub-DB query for last user msg
//   - T-4 (phase-close commit landing) — needs git-log scan
//   - T-5 (hub_session_close call) — at MCP layer (#5)
//   - T-6 spawn-cycle context-check — needs claude-spawn observation
//   - hub_session_load tool + auto-load — #5 N-1(b)-B
//   - index.md maintenance — #6 N-1(b)-C
//   - CLI surface (bot-hq session-analyze) — #5 / #6 / #7
//
// Per Q-III RATIFIED lean (c) hybrid: minimal-create at session-open +
// finalize at session-close. WriteManifest is idempotent — safe to call
// repeatedly with updated fields.
package sessions

import (
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"time"
)

// Manifest is the per-session record per Q-II RATIFIED schema.
// Frontmatter strict (id / project / timestamps / msg-id range / agents
// / pivot markers / parent-session); Body is free-form markdown text.
//
// Optional fields use zero values (empty string / zero time / 0 int);
// renderManifest omits zero-valued optionals from frontmatter output.
type Manifest struct {
	ID              string
	Project         string
	StartTS         time.Time
	EndTS           time.Time
	StartMsgID      int
	EndMsgID        int
	Agents          []string
	PivotInMsgID    int
	PivotOutMsgID   int
	ParentSessionID string
	Body            string
}

// BoundaryTrigger classifies why a session boundary was detected from
// a user message. Surface-only per OQ-1a RATIFIED — caller decides
// whether to fire hub_session_create/_close based on additional context.
type BoundaryTrigger string

const (
	TriggerNone           BoundaryTrigger = ""
	TriggerExplicitPhrase BoundaryTrigger = "T-3-explicit-phrase"
	TriggerRebuildRestart BoundaryTrigger = "T-6-rebuild-restart"
)

// sessionsDirEnvVar overrides the default ~/.bot-hq/sessions/ root.
// Used by tests for R39 TEST-ISOLATION compliance.
const sessionsDirEnvVar = "BOT_HQ_SESSIONS_DIR"

// explicitPhraseRegex implements OQ-1a T-3 RATIFIED set per phase-n.md
// scope-lock v2 §OQ-1a. Case-insensitive; surface-only (probabilistic
// class — caller verifies via explicit hub_session_create call).
var explicitPhraseRegex = regexp.MustCompile(`(?i)\b(let'?s\s+(switch|pivot|move)\s+to|new\s+session|clean\s+slate|new\s+arc|new\s+chapter|EOD|wrap[\s-]?up|done\s+for\s+(today|now)|signing\s+off)\b`)

// rebuildRestartRegex implements OQ-1a T-6 RATIFIED rebuild+restart
// pattern (regex portion). Spawn-cycle context-check (within 60s)
// deferred to caller — needs claude-spawn observation outside this
// package's scope.
var rebuildRestartRegex = regexp.MustCompile(`(?i)\b(rebuild\s*\+\s*restart|rebuild\s+restart|restart\s+session)\b`)

// DetectBoundaryFromUserMsg returns a candidate boundary trigger if the
// user msg matches T-3 (explicit phrase) or T-6 (rebuild+restart)
// patterns. T-6 takes precedence over T-3 when both match (rebuild is
// the more specific event).
//
// Returns TriggerNone if no pattern matches.
func DetectBoundaryFromUserMsg(msg string) BoundaryTrigger {
	if rebuildRestartRegex.MatchString(msg) {
		return TriggerRebuildRestart
	}
	if explicitPhraseRegex.MatchString(msg) {
		return TriggerExplicitPhrase
	}
	return TriggerNone
}

// MakeSessionID returns the canonical session-id string from a date +
// project key per Q-I RATIFIED format YYYY-MM-DD-<project>. Project key
// is lowercased; date in UTC.
//
// Multi-session-same-day suffix (-2 / -3 / etc.) is OQ-2 trivial-fire-
// decision — handled by caller (e.g., append "-2" if same-day session
// already exists in sessions dir).
func MakeSessionID(t time.Time, project string) string {
	return fmt.Sprintf("%s-%s", t.UTC().Format("2006-01-02"), strings.ToLower(project))
}

// SessionsDir returns the canonical sessions-root directory. Override
// via BOT_HQ_SESSIONS_DIR env for R39 TEST-ISOLATION-compliant tests.
func SessionsDir() string {
	if dir := os.Getenv(sessionsDirEnvVar); dir != "" {
		return dir
	}
	home, _ := os.UserHomeDir()
	return filepath.Join(home, ".bot-hq", "sessions")
}

// ManifestPath returns the canonical manifest.md path for a session-id.
// Does not check existence.
func ManifestPath(id string) string {
	return filepath.Join(SessionsDir(), id, "manifest.md")
}

// WriteManifest writes the manifest at SessionsDir/<id>/manifest.md
// with frontmatter (strict) + body (free-form). Idempotent on same
// content; replaces on different content. Per Q-III hybrid lean: safe
// to call at session-open (minimal-create) + session-close (finalize).
func WriteManifest(m Manifest) error {
	if m.ID == "" {
		return fmt.Errorf("manifest ID required")
	}
	dir := filepath.Join(SessionsDir(), m.ID)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return fmt.Errorf("create session dir: %w", err)
	}
	out := renderManifest(m)
	if err := os.WriteFile(ManifestPath(m.ID), []byte(out), 0o644); err != nil {
		return fmt.Errorf("write manifest: %w", err)
	}
	return nil
}

func renderManifest(m Manifest) string {
	var b strings.Builder
	b.WriteString("---\n")
	b.WriteString("id: " + m.ID + "\n")
	if m.Project != "" {
		b.WriteString("project: " + m.Project + "\n")
	}
	if !m.StartTS.IsZero() {
		b.WriteString("start_ts: " + m.StartTS.UTC().Format(time.RFC3339) + "\n")
	}
	if !m.EndTS.IsZero() {
		b.WriteString("end_ts: " + m.EndTS.UTC().Format(time.RFC3339) + "\n")
	}
	if m.StartMsgID > 0 {
		fmt.Fprintf(&b, "start_msg_id: %d\n", m.StartMsgID)
	}
	if m.EndMsgID > 0 {
		fmt.Fprintf(&b, "end_msg_id: %d\n", m.EndMsgID)
	}
	if len(m.Agents) > 0 {
		b.WriteString("agents:\n")
		for _, a := range m.Agents {
			b.WriteString("  - " + a + "\n")
		}
	}
	if m.PivotInMsgID > 0 {
		fmt.Fprintf(&b, "pivot_in_msg_id: %d\n", m.PivotInMsgID)
	}
	if m.PivotOutMsgID > 0 {
		fmt.Fprintf(&b, "pivot_out_msg_id: %d\n", m.PivotOutMsgID)
	}
	if m.ParentSessionID != "" {
		b.WriteString("parent_session_id: " + m.ParentSessionID + "\n")
	}
	b.WriteString("---\n\n")
	if m.Body != "" {
		b.WriteString(m.Body)
		if !strings.HasSuffix(m.Body, "\n") {
			b.WriteString("\n")
		}
	}
	return b.String()
}

// ReadManifest reads + parses the manifest at SessionsDir/<id>/manifest
// .md. Symmetric inverse of WriteManifest — frontmatter fields populated
// + body preserved.
//
// Returns os-class errors verbatim from os.ReadFile (file-not-found
// callers can check os.IsNotExist) and parse-class errors with
// "manifest:" prefix.
func ReadManifest(id string) (Manifest, error) {
	data, err := os.ReadFile(ManifestPath(id))
	if err != nil {
		return Manifest{}, err
	}
	return parseManifest(string(data))
}

func parseManifest(content string) (Manifest, error) {
	var m Manifest
	if !strings.HasPrefix(content, "---\n") {
		return m, fmt.Errorf("manifest: missing frontmatter open marker")
	}
	rest := strings.TrimPrefix(content, "---\n")
	end := strings.Index(rest, "\n---\n")
	if end < 0 {
		return m, fmt.Errorf("manifest: missing frontmatter close marker")
	}
	frontmatter := rest[:end]
	body := strings.TrimPrefix(rest[end:], "\n---\n")
	body = strings.TrimPrefix(body, "\n")
	m.Body = body

	inAgents := false
	for _, line := range strings.Split(frontmatter, "\n") {
		if line == "" {
			inAgents = false
			continue
		}
		if inAgents {
			if strings.HasPrefix(line, "  - ") {
				m.Agents = append(m.Agents, strings.TrimPrefix(line, "  - "))
				continue
			}
			inAgents = false
		}
		switch {
		case strings.HasPrefix(line, "id: "):
			m.ID = strings.TrimPrefix(line, "id: ")
		case strings.HasPrefix(line, "project: "):
			m.Project = strings.TrimPrefix(line, "project: ")
		case strings.HasPrefix(line, "start_ts: "):
			m.StartTS, _ = time.Parse(time.RFC3339, strings.TrimPrefix(line, "start_ts: "))
		case strings.HasPrefix(line, "end_ts: "):
			m.EndTS, _ = time.Parse(time.RFC3339, strings.TrimPrefix(line, "end_ts: "))
		case strings.HasPrefix(line, "start_msg_id: "):
			fmt.Sscanf(strings.TrimPrefix(line, "start_msg_id: "), "%d", &m.StartMsgID)
		case strings.HasPrefix(line, "end_msg_id: "):
			fmt.Sscanf(strings.TrimPrefix(line, "end_msg_id: "), "%d", &m.EndMsgID)
		case line == "agents:":
			inAgents = true
		case strings.HasPrefix(line, "pivot_in_msg_id: "):
			fmt.Sscanf(strings.TrimPrefix(line, "pivot_in_msg_id: "), "%d", &m.PivotInMsgID)
		case strings.HasPrefix(line, "pivot_out_msg_id: "):
			fmt.Sscanf(strings.TrimPrefix(line, "pivot_out_msg_id: "), "%d", &m.PivotOutMsgID)
		case strings.HasPrefix(line, "parent_session_id: "):
			m.ParentSessionID = strings.TrimPrefix(line, "parent_session_id: ")
		}
	}
	return m, nil
}
