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
	"sort"
	"strconv"
	"strings"
	"time"
)

// Manifest is the per-session record per Q-II RATIFIED schema.
// Frontmatter strict (id / project / timestamps / msg-id range / agents
// / pivot markers / parent-session); Body is free-form markdown text.
//
// Optional fields use zero values (empty string / zero time / 0 int);
// renderManifest omits zero-valued optionals from frontmatter output.
//
// Phase R R5 (d-2) extension — checkpoint fields:
// ActiveWorkstream / LastCommitSHA / Phase / Posture / CheckpointTS.
// Written via hub_session_checkpoint MCP tool + WriteCheckpoint helper
// at boundary moments (phase-open / commit-land / pivot / etc.).
// Backwards-compat per Refine-B: zero-valued checkpoint fields omit
// from frontmatter output; existing pre-R5 manifests round-trip
// cleanly (parseManifest skips missing-field lines).
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
	// Phase R R5 (d-2) checkpoint fields
	ActiveWorkstream string
	LastCommitSHA    string
	Phase            string
	Posture          string
	CheckpointTS     time.Time
	Body             string
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

// capitalKeywordRegex implements Phase S S-3 capital-keyword
// boundary triggers per user msg 15734 item-3 + msg 15753 OQ-S5
// (formal session-close on `DONE` / `PIVOT`). Case-sensitive
// uppercase-only to distinguish from prose use of "done" / "pivot"
// (e.g., "I'm done now" / "let's pivot the strategy"). Folded into
// TriggerExplicitPhrase semantic alongside the existing T-3 set.
var capitalKeywordRegex = regexp.MustCompile(`\b(DONE|PIVOT)\b`)

// DetectBoundaryFromUserMsg returns a candidate boundary trigger if the
// user msg matches T-3 (explicit phrase, including Phase S capital-
// keyword DONE/PIVOT extension) or T-6 (rebuild+restart) patterns.
// T-6 takes precedence over T-3 when both match (rebuild is the more
// specific event).
//
// Returns TriggerNone if no pattern matches.
func DetectBoundaryFromUserMsg(msg string) BoundaryTrigger {
	if rebuildRestartRegex.MatchString(msg) {
		return TriggerRebuildRestart
	}
	if explicitPhraseRegex.MatchString(msg) || capitalKeywordRegex.MatchString(msg) {
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
//
// Per phase-p.md §P-6 (OQ-6 secrets-scan-on-manifest-author): rendered
// content is scanned via ScanForSecrets; findings are appended to the
// manifest-secrets log (best-effort; log failures do NOT fail the write).
// When BOT_HQ_SECRETS_STRICT=1 is set, findings return an error instead
// of logging — caller decides retry/redact policy.
func WriteManifest(m Manifest) error {
	if m.ID == "" {
		return fmt.Errorf("manifest ID required")
	}
	dir := filepath.Join(SessionsDir(), m.ID)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return fmt.Errorf("create session dir: %w", err)
	}
	out := renderManifest(m)
	if findings := ScanForSecrets(out); len(findings) > 0 {
		LogSecretFindings(m.ID, findings)
		if secretsStrictMode() {
			return fmt.Errorf("manifest %s: %d secret-pattern hit(s) detected (BOT_HQ_SECRETS_STRICT=1; redact + retry)", m.ID, len(findings))
		}
	}
	// Atomic write per Phase R R5 (d-2) Refine-C correctness: write to
	// temp file in same dir, then os.Rename — atomic on POSIX same-fs.
	// Prevents partial-write corruption on crash mid-checkpoint.
	target := ManifestPath(m.ID)
	tmp := target + ".tmp"
	if err := os.WriteFile(tmp, []byte(out), 0o644); err != nil {
		return fmt.Errorf("write manifest tmp: %w", err)
	}
	if err := os.Rename(tmp, target); err != nil {
		_ = os.Remove(tmp)
		return fmt.Errorf("rename manifest: %w", err)
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
	// Phase R R5 (d-2) checkpoint fields (omit zero-values per Refine-B
	// backwards-compat).
	if m.ActiveWorkstream != "" {
		b.WriteString("active_workstream: " + m.ActiveWorkstream + "\n")
	}
	if m.LastCommitSHA != "" {
		b.WriteString("last_commit_sha: " + m.LastCommitSHA + "\n")
	}
	if m.Phase != "" {
		b.WriteString("phase: " + m.Phase + "\n")
	}
	if m.Posture != "" {
		b.WriteString("posture: " + m.Posture + "\n")
	}
	if !m.CheckpointTS.IsZero() {
		b.WriteString("checkpoint_ts: " + m.CheckpointTS.UTC().Format(time.RFC3339) + "\n")
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

// IndexPath returns the canonical rolling index path
// SessionsDir/index.md per N-1 (a) §4 storage shape.
func IndexPath() string {
	return filepath.Join(SessionsDir(), "index.md")
}

// indexEntry is the per-row payload for WriteIndex; held as a named
// type so sort.Slice can index without anonymous-struct boilerplate.
type indexEntry struct {
	id       string
	manifest Manifest
}

// WriteIndex rebuilds the rolling sessions index at IndexPath() by
// scanning all session-id directories under SessionsDir and grouping
// entries by project. Greppable single-line per-session format:
//
//	- <id> | <start_ts> | <end_ts-or-active> | <agents-csv> | <project>
//
// Sections grouped by project (## <project>); within each section,
// entries sorted by id descending (most-recent first per Q-V auto-
// load semantics).
//
// Idempotent — safe to call repeatedly. Per N-1 (a) §6 step 4
// "session-create/close updates index.md rolling list".
func WriteIndex() error {
	ids, err := ListSessionIDs()
	if err != nil {
		return fmt.Errorf("list sessions: %w", err)
	}
	byProject := map[string][]indexEntry{}
	for _, id := range ids {
		m, err := ReadManifest(id)
		if err != nil {
			continue
		}
		byProject[m.Project] = append(byProject[m.Project], indexEntry{id, m})
	}

	var b strings.Builder
	b.WriteString("# Bot-HQ Sessions Index\n\n")
	b.WriteString("Rolling list of session-clusters per N-1 (a) §4 storage shape. Sorted most-recent-first within each project section.\n\n")

	projects := make([]string, 0, len(byProject))
	for p := range byProject {
		projects = append(projects, p)
	}
	sort.Strings(projects)

	for _, project := range projects {
		entries := byProject[project]
		sort.Slice(entries, func(i, j int) bool { return entries[i].id > entries[j].id })
		title := project
		if title == "" {
			title = "(no project)"
		}
		fmt.Fprintf(&b, "## %s\n\n", title)
		for _, e := range entries {
			startTS := "active"
			if !e.manifest.StartTS.IsZero() {
				startTS = e.manifest.StartTS.UTC().Format(time.RFC3339)
			}
			endTS := "active"
			if !e.manifest.EndTS.IsZero() {
				endTS = e.manifest.EndTS.UTC().Format(time.RFC3339)
			}
			agents := strings.Join(e.manifest.Agents, ",")
			fmt.Fprintf(&b, "- %s | %s | %s | %s | %s\n", e.id, startTS, endTS, agents, project)
		}
		b.WriteString("\n")
	}

	if err := os.MkdirAll(SessionsDir(), 0o755); err != nil {
		return fmt.Errorf("ensure sessions dir: %w", err)
	}
	return os.WriteFile(IndexPath(), []byte(b.String()), 0o644)
}

// LoadManifestContent reads the raw manifest.md content for a session-id.
// Helper for hub_session_load MCP tool + CLI surface — returns the
// frontmatter+body bytes verbatim (consumer can re-parse via
// ReadManifest if structured access is needed).
func LoadManifestContent(id string) (string, error) {
	data, err := os.ReadFile(ManifestPath(id))
	if err != nil {
		return "", err
	}
	return string(data), nil
}

// ListSessionIDs returns the session-ids present under SessionsDir.
// Per N-1 (a) §4 storage shape: each <id>/ is a session directory.
// Skips files (only dirs counted) + skips index.md (Phase N v2 #6
// scope). Returns empty slice (not error) when SessionsDir does not
// exist yet.
func ListSessionIDs() ([]string, error) {
	dir := SessionsDir()
	entries, err := os.ReadDir(dir)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}
	var ids []string
	for _, e := range entries {
		if !e.IsDir() {
			continue
		}
		ids = append(ids, e.Name())
	}
	return ids, nil
}

// MostRecentForProject returns the most-recent session-id matching the
// given project key. Lexicographic sort on session-id (which begins
// with YYYY-MM-DD) ensures most-recent = max id.
//
// Returns empty string + nil error when no matching session exists.
// Caller distinguishes empty-result vs error.
//
// Used by retention auto-load on hub_register (Phase N v2 follow-up;
// MVP exposes the helper, integration deferred per Q1 scope-trim).
func MostRecentForProject(project string) (string, error) {
	ids, err := ListSessionIDs()
	if err != nil {
		return "", err
	}
	suffix := "-" + strings.ToLower(project)
	var best string
	for _, id := range ids {
		if !strings.HasSuffix(id, suffix) {
			continue
		}
		if id > best {
			best = id
		}
	}
	return best, nil
}

// DefaultRetentionDays is the default retention window applied by Prune
// when no caller-specific override is supplied. 30 days reflects a
// conservative "long enough for retro context, short enough that disk
// growth stays bounded" trade-off per phase-p.md §P-5 acceptance.
const DefaultRetentionDays = 30

// pickAge returns the most-recent timestamp from a manifest for the
// retention-window comparison. Prefers EndTS (canonical close-time);
// falls back to StartTS when EndTS is zero (manifest authored at open
// but never closed — orphaned session).
func pickAge(m Manifest) time.Time {
	if !m.EndTS.IsZero() {
		return m.EndTS
	}
	return m.StartTS
}

// IsWithinRetention reports whether the given session-id is younger
// than retentionDays as of `now`. Returns (false, nil) when the
// session manifest can't be parsed or has no usable timestamps —
// caller treats unknown-age as out-of-window for safety.
//
// retentionDays <= 0 disables the window check (always within).
//
// Used by hub_session_load auto-load decisions per Phase N v2 #5
// follow-up: retention auto-load should only fire if last session
// is recent enough to be useful resume context.
func IsWithinRetention(id string, retentionDays int, now time.Time) (bool, error) {
	if retentionDays <= 0 {
		return true, nil
	}
	m, err := ReadManifest(id)
	if err != nil {
		if os.IsNotExist(err) {
			return false, nil
		}
		return false, err
	}
	age := pickAge(m)
	if age.IsZero() {
		return false, nil
	}
	cutoff := now.Add(-time.Duration(retentionDays) * 24 * time.Hour)
	return age.After(cutoff), nil
}

// PruneOlderThan deletes session directories whose latest manifest
// timestamp is older than retentionDays as of `now`. Returns the slice
// of pruned session-ids in deletion order. Sessions with unparseable
// manifests are skipped (logged via the returned error chain only on
// fatal-class issues; missing-manifest sessions are skipped silently).
//
// retentionDays <= 0 is a safety no-op (never prune; returns nil).
//
// Per phase-p.md §P-5 acceptance: configurable retention window +
// nightly/on-demand prune. CLI subcommand `bot-hq session-prune` wraps
// this for operator-driven invocations; future cron / scheduled-task
// integration can call this directly in-process.
func PruneOlderThan(retentionDays int, now time.Time) ([]string, error) {
	if retentionDays <= 0 {
		return nil, nil
	}
	ids, err := ListSessionIDs()
	if err != nil {
		return nil, err
	}
	cutoff := now.Add(-time.Duration(retentionDays) * 24 * time.Hour)
	var pruned []string
	dir := SessionsDir()
	for _, id := range ids {
		m, err := ReadManifest(id)
		if err != nil {
			// Manifest unreadable / unparseable: skip rather than
			// remove — operator can investigate manually before any
			// cleanup. Conservative-by-default per data-loss-class.
			continue
		}
		age := pickAge(m)
		if age.IsZero() {
			continue
		}
		if age.Before(cutoff) {
			if err := os.RemoveAll(filepath.Join(dir, id)); err != nil {
				return pruned, fmt.Errorf("remove %s: %w", id, err)
			}
			pruned = append(pruned, id)
		}
	}
	return pruned, nil
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
		// Phase R R5 (d-2) checkpoint fields. Pre-R5 manifests don't
		// have these lines and skip silently per Refine-B backwards-compat.
		case strings.HasPrefix(line, "active_workstream: "):
			m.ActiveWorkstream = strings.TrimPrefix(line, "active_workstream: ")
		case strings.HasPrefix(line, "last_commit_sha: "):
			m.LastCommitSHA = strings.TrimPrefix(line, "last_commit_sha: ")
		case strings.HasPrefix(line, "phase: "):
			m.Phase = strings.TrimPrefix(line, "phase: ")
		case strings.HasPrefix(line, "posture: "):
			m.Posture = strings.TrimPrefix(line, "posture: ")
		case strings.HasPrefix(line, "checkpoint_ts: "):
			m.CheckpointTS, _ = time.Parse(time.RFC3339, strings.TrimPrefix(line, "checkpoint_ts: "))
		}
	}
	return m, nil
}

// CheckpointFields carries the fields written by hub_session_checkpoint.
// Empty fields signal "leave existing value unchanged" — WriteCheckpoint
// reads the current manifest and only overwrites non-empty fields, so
// callers can update a single field without supplying the rest.
//
// Phase R R5 (d-2) per phase-r.md R5 cluster + msg 15504 lean (iii).
type CheckpointFields struct {
	ActiveWorkstream string
	LastCommitSHA    string
	Phase            string
	Posture          string
	BodyAppend       string // optional: appended to manifest body if set
}

// WriteCheckpoint reads the manifest at SessionsDir/<id>/manifest.md,
// merges non-empty checkpoint fields into it, refreshes CheckpointTS,
// optionally appends BodyAppend to the body with a timestamped header,
// and writes back. Idempotent across no-op checkpoint calls.
//
// Returns os.ErrNotExist if the manifest doesn't exist (caller can
// distinguish via os.IsNotExist).
//
// Phase R R5 (d-2) per phase-r.md R5 cluster.
func WriteCheckpoint(id string, cp CheckpointFields) error {
	m, err := ReadManifest(id)
	if err != nil {
		return err
	}
	if cp.ActiveWorkstream != "" {
		m.ActiveWorkstream = cp.ActiveWorkstream
	}
	if cp.LastCommitSHA != "" {
		m.LastCommitSHA = cp.LastCommitSHA
	}
	if cp.Phase != "" {
		m.Phase = cp.Phase
	}
	if cp.Posture != "" {
		m.Posture = cp.Posture
	}
	m.CheckpointTS = time.Now().UTC()
	if cp.BodyAppend != "" {
		header := fmt.Sprintf("\n## Checkpoint %s\n\n", m.CheckpointTS.Format(time.RFC3339))
		m.Body = m.Body + header + cp.BodyAppend
		if !strings.HasSuffix(m.Body, "\n") {
			m.Body = m.Body + "\n"
		}
	}
	return WriteManifest(m)
}

// citedMsgIDPattern matches `msg <N>` and `msg-<N>` patterns used as
// cite-anchors throughout discipline-log / phase / arc / ratchet docs.
// Phase R R5 (d-3) cite-anchor preservation per Rain msg 15545
// Refine-C 7-path-classes scope.
//
// Pattern: word "msg" (case-insensitive) followed by optional space/dash
// then 4+ decimal digits. Captures: full match + msg-id integer.
//
// Hyphen-list "msg 5194-5218" matches "msg 5194" once; the second
// number is treated as a follow-up endpoint via a separate extraction
// path inside ScanCitedMsgIDs.
var citedMsgIDPattern = regexp.MustCompile(`(?i)\bmsg[\s-]+(\d{3,})(?:[-/](\d{3,}))?`)

// CiteAnchorScanRoots returns the canonical 7 path-classes scanned for
// `msg <N>` cite-anchors per Phase R R5 (d-3) Refine-C. Resolves at
// call-time to ${HOME}-relative paths; tests can override by passing
// canonRoot + repoRoot directly into ScanCitedMsgIDs.
func CiteAnchorScanRoots(canonRoot, repoRoot string) []string {
	return []string{
		filepath.Join(canonRoot, "discipline-log.md"),
		filepath.Join(canonRoot, "phase"),
		filepath.Join(canonRoot, "ratchets"),
		filepath.Join(canonRoot, "projects"),
		filepath.Join(canonRoot, "brian", "discipline-anchors.md"),
		filepath.Join(canonRoot, "rain", "discipline-anchors.md"),
		filepath.Join(repoRoot, "docs", "arcs"),
	}
}

// ScanCitedMsgIDs scans the canonical 7 path-classes for `msg <N>`
// cite-anchor references and returns the de-duplicated set of msg-IDs
// found. Phase R R5 (d-3) cite-anchor preservation per Rain msg 15545
// Refine-C scope expansion.
//
// canonRoot is typically ~/.bot-hq/; repoRoot is typically
// ~/Projects/bot-hq/. Both must be absolute paths.
//
// Returns empty map (not nil) on success with zero matches; non-nil
// error only on fatal IO failure during walk.
func ScanCitedMsgIDs(canonRoot, repoRoot string) (map[int]bool, error) {
	cited := map[int]bool{}
	roots := CiteAnchorScanRoots(canonRoot, repoRoot)

	for _, root := range roots {
		info, err := os.Stat(root)
		if err != nil {
			if os.IsNotExist(err) {
				continue
			}
			return cited, fmt.Errorf("stat %s: %w", root, err)
		}
		if !info.IsDir() {
			if err := scanFileForMsgIDs(root, cited); err != nil {
				return cited, err
			}
			continue
		}
		err = filepath.Walk(root, func(path string, fi os.FileInfo, walkErr error) error {
			if walkErr != nil {
				return walkErr
			}
			if fi.IsDir() {
				return nil
			}
			if !strings.HasSuffix(path, ".md") {
				return nil
			}
			return scanFileForMsgIDs(path, cited)
		})
		if err != nil {
			return cited, fmt.Errorf("walk %s: %w", root, err)
		}
	}
	return cited, nil
}

func scanFileForMsgIDs(path string, cited map[int]bool) error {
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return nil
		}
		return fmt.Errorf("read %s: %w", path, err)
	}
	for _, m := range citedMsgIDPattern.FindAllStringSubmatch(string(data), -1) {
		if id, err := strconv.Atoi(m[1]); err == nil {
			cited[id] = true
		}
		if len(m) >= 3 && m[2] != "" {
			if id, err := strconv.Atoi(m[2]); err == nil {
				cited[id] = true
			}
		}
	}
	return nil
}

// SessionRangeOverlapsCited returns true when the session manifest's
// [StartMsgID, EndMsgID] range contains any cited msg-id. Used by
// PruneOlderThanWithCitePreservation to exempt cited sessions from
// retention purge.
//
// When EndMsgID is zero (session opened but never closed → no
// upper bound), only StartMsgID is checked. When StartMsgID is also
// zero (no msg-id range recorded), no overlap can be determined →
// returns false (session is purgeable per retention window).
func SessionRangeOverlapsCited(m Manifest, cited map[int]bool) bool {
	if len(cited) == 0 {
		return false
	}
	if m.StartMsgID == 0 {
		return false
	}
	end := m.EndMsgID
	if end == 0 {
		end = m.StartMsgID
	}
	for id := range cited {
		if id >= m.StartMsgID && id <= end {
			return true
		}
	}
	return false
}

// PruneOlderThanWithCitePreservation extends PruneOlderThan with
// cite-anchor preservation per Phase R R5 (d-3). Sessions whose
// msg-id range overlaps any cited msg-id (from the 7 scan roots
// resolved via canonRoot+repoRoot) are EXEMPTED from purge regardless
// of age. All other PruneOlderThan semantics preserved.
//
// canonRoot/repoRoot empty strings → resolves from $HOME defaults
// (~/.bot-hq/ + ~/Projects/bot-hq/).
//
// Returns (prunedIDs, exemptedCitedIDs, error).
func PruneOlderThanWithCitePreservation(retentionDays int, now time.Time, canonRoot, repoRoot string) ([]string, []string, error) {
	if retentionDays <= 0 {
		return nil, nil, nil
	}
	if canonRoot == "" {
		home, _ := os.UserHomeDir()
		canonRoot = filepath.Join(home, ".bot-hq")
	}
	if repoRoot == "" {
		home, _ := os.UserHomeDir()
		repoRoot = filepath.Join(home, "Projects", "bot-hq")
	}
	cited, err := ScanCitedMsgIDs(canonRoot, repoRoot)
	if err != nil {
		return nil, nil, fmt.Errorf("scan cited msg-ids: %w", err)
	}
	ids, err := ListSessionIDs()
	if err != nil {
		return nil, nil, err
	}
	cutoff := now.Add(-time.Duration(retentionDays) * 24 * time.Hour)
	var pruned, exempted []string
	dir := SessionsDir()
	for _, id := range ids {
		m, err := ReadManifest(id)
		if err != nil {
			continue
		}
		age := pickAge(m)
		if age.IsZero() {
			continue
		}
		if !age.Before(cutoff) {
			continue
		}
		if SessionRangeOverlapsCited(m, cited) {
			exempted = append(exempted, id)
			continue
		}
		if err := os.RemoveAll(filepath.Join(dir, id)); err != nil {
			return pruned, exempted, fmt.Errorf("remove %s: %w", id, err)
		}
		pruned = append(pruned, id)
	}
	return pruned, exempted, nil
}
