// Phase W close-summary extraction + finalize machinery.
//
// Finalize() takes an active session manifest and writes a closed
// version with rich retrospective payload: outcome narrative
// (caller-supplied), MsgCount + Decisions (extracted from hub
// messages), CommitsLanded + FilesTouched (extracted from git log
// when repoPath is supplied).
//
// Design split: extraction helpers are pure functions over their
// input (messages slice / repo path). Finalize wraps them with the
// manifest read/write + index rebuild. This keeps the helpers
// testable without requiring DB / filesystem fakes.

package sessions

import (
	"fmt"
	"os/exec"
	"regexp"
	"strings"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// decisionKeywordPattern matches the canonical decision-class signals
// the duo uses in hub messages. Case-insensitive — the keywords are
// canonical-form in disc.go but some prose / summary content uses
// lowercase variants.
var decisionKeywordPattern = regexp.MustCompile(`(?i)\b(BRAIN-AGREED|GREENFLAG|HALT|SCOPE-LOCK)\b`)

// ExtractDecisionsFromMessages scans the given messages for
// decision-class events and returns formatted entries.
//
// Each entry: "<msg-id>: <KIND> | <gloss>" where KIND is the matched
// keyword (uppercased) and gloss is the first ~80 chars of content.
//
// Pure function — no DB / filesystem access. Caller supplies the
// already-windowed message slice.
func ExtractDecisionsFromMessages(msgs []protocol.Message) []string {
	var out []string
	for _, m := range msgs {
		match := decisionKeywordPattern.FindString(m.Content)
		if match == "" {
			continue
		}
		gloss := strings.ReplaceAll(strings.TrimSpace(m.Content), "\n", " ")
		if len(gloss) > 80 {
			gloss = gloss[:80] + "..."
		}
		out = append(out, fmt.Sprintf("%d: %s | %s", m.ID, strings.ToUpper(match), gloss))
	}
	return out
}

// ExtractGitChanges runs `git log` against repoPath for commits in the
// [since, until] window. Returns the SHA list + de-duplicated touched
// file list.
//
// Both git invocations use ISO-8601 timestamps. Empty repoPath returns
// (nil, nil, nil) — caller's responsibility to pass an empty path when
// the project doesn't have a known local clone.
//
// Errors from git (repo not found, etc) propagate; caller decides
// whether to skip-on-error or abort the close.
func ExtractGitChanges(repoPath string, since, until time.Time) ([]string, []string, error) {
	if strings.TrimSpace(repoPath) == "" {
		return nil, nil, nil
	}
	sinceArg := since.Format(time.RFC3339)
	untilArg := until.Format(time.RFC3339)

	// SHAs first
	shaCmd := exec.Command("git", "-C", repoPath, "log",
		"--since", sinceArg,
		"--until", untilArg,
		"--pretty=format:%H")
	shaOut, err := shaCmd.Output()
	if err != nil {
		return nil, nil, fmt.Errorf("git log SHA: %w", err)
	}
	var commits []string
	for _, line := range strings.Split(strings.TrimSpace(string(shaOut)), "\n") {
		line = strings.TrimSpace(line)
		if line != "" {
			commits = append(commits, line)
		}
	}

	// Touched files: git log --name-only with empty --pretty so output
	// is just file paths, one per line, deduplicated client-side.
	filesCmd := exec.Command("git", "-C", repoPath, "log",
		"--since", sinceArg,
		"--until", untilArg,
		"--name-only",
		"--pretty=format:")
	filesOut, err := filesCmd.Output()
	if err != nil {
		return commits, nil, fmt.Errorf("git log files: %w", err)
	}
	seen := map[string]bool{}
	var files []string
	for _, line := range strings.Split(string(filesOut), "\n") {
		line = strings.TrimSpace(line)
		if line == "" || seen[line] {
			continue
		}
		seen[line] = true
		files = append(files, line)
	}
	return commits, files, nil
}

// FinalizeOptions carry the inputs for Finalize() that aren't on the
// manifest itself.
type FinalizeOptions struct {
	Outcome string // agent-authored narrative (required)
	Status  string // defaults to "closed" if empty
	Now     time.Time
	// Messages is the hub message window between StartMsgID and the
	// current latest. Caller queries db.GetMessagesFromAgent or similar
	// and passes the result. Empty slice → no decisions extracted.
	Messages []protocol.Message
	// RepoPath is the git repo root for CommitsLanded / FilesTouched
	// extraction. Empty → skip git extraction.
	RepoPath string
	// LatestMsgID — used to compute MsgCount and EndMsgID. Caller
	// supplies via hub.DB query (`GetRecentMessages(1)[0].ID` or
	// equivalent) so Finalize stays DB-agnostic.
	LatestMsgID int
}

// Finalize closes the session manifest at the given id with rich
// close-summary payload. Reads the existing manifest, populates the
// Phase W fields, and writes back atomically. Idempotent — re-running
// Finalize on an already-closed session updates EndTS / Outcome but
// preserves the existing close metadata when opts are empty.
//
// The caller-supplied Outcome is required; an empty Outcome returns
// an error so look-back narrative is never lost. Use a synthetic
// outcome (e.g., "auto-migrated; no in-flight work") when finalizing
// a stale-active session retroactively.
func Finalize(id string, opts FinalizeOptions) (Manifest, error) {
	if strings.TrimSpace(opts.Outcome) == "" {
		return Manifest{}, fmt.Errorf("Finalize: outcome required (synthetic okay for migration)")
	}

	m, err := ReadManifest(id)
	if err != nil {
		return Manifest{}, fmt.Errorf("read manifest: %w", err)
	}

	now := opts.Now
	if now.IsZero() {
		now = time.Now().UTC()
	}
	m.EndTS = now.UTC()

	if strings.TrimSpace(opts.Status) == "" {
		m.Status = "closed"
	} else {
		m.Status = opts.Status
	}

	m.Outcome = opts.Outcome

	if opts.LatestMsgID > 0 {
		m.EndMsgID = opts.LatestMsgID
		if m.StartMsgID > 0 && opts.LatestMsgID >= m.StartMsgID {
			m.MsgCount = opts.LatestMsgID - m.StartMsgID
		}
	}

	if len(opts.Messages) > 0 {
		m.Decisions = ExtractDecisionsFromMessages(opts.Messages)
	}

	if opts.RepoPath != "" {
		commits, files, gitErr := ExtractGitChanges(opts.RepoPath, m.StartTS, m.EndTS)
		if gitErr != nil {
			// Non-fatal — continue with the close, log via returned manifest's
			// body so retrospective can see why git extraction was missing.
			m.Body += fmt.Sprintf("\n_(git extraction skipped: %v)_\n", gitErr)
		} else {
			m.CommitsLanded = commits
			m.FilesTouched = files
		}
	}

	if err := WriteManifest(m); err != nil {
		return Manifest{}, fmt.Errorf("write manifest: %w", err)
	}
	if err := WriteIndex(); err != nil {
		// Index rebuild failure is non-fatal — manifest is the source of
		// truth, index is derived. Log via returned manifest, caller decides.
		m.Body += fmt.Sprintf("\n_(index rebuild failed: %v)_\n", err)
	}
	return m, nil
}

// FindActiveForProject returns the session-id of the most-recent
// session for the given project that has not yet been closed
// (EndTS zero). Returns empty string + nil error when no active
// session exists for the project.
//
// Used by hub_session_create to detect cross-project active sessions
// (pivot enforcement) and by hub_session_finalize to default the
// target session.
func FindActiveForProject(project string) (string, error) {
	ids, err := ListSessionIDs()
	if err != nil {
		return "", err
	}
	suffix := strings.ToLower(project)
	var best string
	for _, id := range ids {
		// Match either "YYYY-MM-DD-<project>" or "YYYY-MM-DD-<project>-N"
		// — the project key is the trailing token after the date prefix.
		parts := strings.SplitN(id, "-", 4)
		if len(parts) < 4 {
			continue
		}
		// parts[3] is "<project>" or "<project>-N"; strip optional -N.
		projectPart := parts[3]
		if dash := strings.LastIndexByte(projectPart, '-'); dash >= 0 {
			tail := projectPart[dash+1:]
			if isAllDigits(tail) {
				projectPart = projectPart[:dash]
			}
		}
		if !strings.EqualFold(projectPart, suffix) {
			continue
		}
		m, err := ReadManifest(id)
		if err != nil {
			continue
		}
		if !m.EndTS.IsZero() {
			continue // closed
		}
		if id > best {
			best = id
		}
	}
	return best, nil
}

// FindActiveForAnyOtherProject returns the session-id of the most-recent
// active session that does NOT match the given project. Used by
// hub_session_create to detect cross-project pivots that need to
// finalize the prior project's session first.
//
// Returns ("", "", nil) when no other-project active session exists.
// Otherwise returns (sessionID, projectName, nil).
func FindActiveForAnyOtherProject(currentProject string) (string, string, error) {
	ids, err := ListSessionIDs()
	if err != nil {
		return "", "", err
	}
	current := strings.ToLower(currentProject)
	var bestID, bestProject string
	for _, id := range ids {
		m, err := ReadManifest(id)
		if err != nil {
			continue
		}
		if !m.EndTS.IsZero() {
			continue
		}
		if strings.EqualFold(m.Project, current) {
			continue
		}
		if id > bestID {
			bestID = id
			bestProject = m.Project
		}
	}
	return bestID, bestProject, nil
}

func isAllDigits(s string) bool {
	if s == "" {
		return false
	}
	for _, r := range s {
		if r < '0' || r > '9' {
			return false
		}
	}
	return true
}
