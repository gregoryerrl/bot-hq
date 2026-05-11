package protocol

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"time"
)

// AgentState is the durable per-agent state-anchor for the
// BOOTSTRAP-ON-CONVERSATION-RESUME (R20) discipline. Persisted to
// `~/.bot-hq/<agentID>/last_state.json` (or under BOT_HQ_HOME if set).
//
// Phase J T1.5 (B1d) — feeds R20's reactive detection of
// context-discontinuity. Together with B1(v) CLAUDE.md Compact
// Instructions (T1.1, proactive survival), forms defense-in-depth
// against summary-fragmentation drift (e.g., the BCC-into-bot-hq
// drift class observed Phase I).
//
// Schema-lock target: TestAgentStateSchema (agentstate_test.go) asserts
// JSON output contains the exact keys below; accidental field removal
// or rename fails CI before landing.
//
// Design source: docs/plans/2026-04-29-bootstrap-resume-design.md §5.4
// (Option D-A primary).
type AgentState struct {
	// LastSelfMsgID is the agent's most-recent successful hub_send msg-id.
	// Compared at scope-affecting-turn-start against in-context history;
	// absence → context-discontinuity → R16 bootstrap.
	LastSelfMsgID int64 `json:"last_self_msg_id"`

	// LastCommitSHA is the git tip of the agent's primary work-repo at
	// the moment of last AgentState write. Cross-validates against
	// `git log -1` at bootstrap time.
	LastCommitSHA string `json:"last_commit_sha"`

	// LastPhaseDoc is the basename of the active scope-lock doc
	// (e.g., "phase-j.md"). Cross-validates against
	// ~/.bot-hq/projects/bot-hq/phase/<active>.md presence at bootstrap.
	LastPhaseDoc string `json:"last_phase_doc"`

	// LastRatchetPull is when the agent last read
	// ~/.bot-hq/projects/bot-hq/ratchets/active.md.
	LastRatchetPull time.Time `json:"last_ratchet_pull_at"`

	// LastStateWrite is when this AgentState was written. Stale state
	// (>1h delta from now()) suggests skipped writes; agent flags
	// missing-write-discipline at bootstrap.
	LastStateWrite time.Time `json:"last_state_write_at"`

	// DisciplineAnchorSHA256 is the SHA-256 hex digest of the agent's
	// ~/.bot-hq/<agentID>/discipline-anchors.md file at the moment of
	// last AgentState write. R16 bootstrap recomputes the current SHA
	// and compares — mismatch indicates the agent's loaded discipline
	// context drifted (e.g., post-autocompact or post-anchor-edit
	// without re-read) and triggers the K-17 mutual-halt protocol.
	//
	// Phase K K-12 — closes the autocompact-induced discipline-drift
	// failure class observed bcc-ad-manager session 2026-04-29.
	DisciplineAnchorSHA256 string `json:"discipline_anchor_sha256"`
}

// agentStateDir returns the per-agent state directory honoring
// BOT_HQ_HOME for tests. Mirrors sentinel.go's sentinelsDir pattern.
func agentStateDir(agentID string) (string, error) {
	if home := os.Getenv("BOT_HQ_HOME"); home != "" {
		return filepath.Join(home, agentID), nil
	}
	homeDir, err := os.UserHomeDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(homeDir, ".bot-hq", agentID), nil
}

// agentStatePath returns the full path to the per-agent last_state.json.
func agentStatePath(agentID string) (string, error) {
	dir, err := agentStateDir(agentID)
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, "last_state.json"), nil
}

// WriteAgentState persists state to the agent's per-agent file.
// Creates the directory if absent. Atomic via temp-file + rename to
// survive concurrent writes / crash mid-write.
//
// Caller responsibility: invoke after every successful scope-relevant
// hub_send (commit-narration, BRAIN-cycle-decision, scope-change). The
// rule R20 governs when; this helper governs how.
func WriteAgentState(agentID string, state AgentState) error {
	path, err := agentStatePath(agentID)
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return err
	}
	state.LastStateWrite = time.Now().UTC()
	data, err := json.MarshalIndent(state, "", "  ")
	if err != nil {
		return err
	}
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, data, 0o644); err != nil {
		return err
	}
	return os.Rename(tmp, path)
}

// ReadAgentState reads state from the agent's per-agent file. Returns
// (zero-value, nil) if file absent (first-ever-call path is not an
// error). Returns (zero-value, err) on parse error.
//
// Caller responsibility: invoke at scope-affecting-turn-start; compare
// LastSelfMsgID against in-context history per R20.
func ReadAgentState(agentID string) (AgentState, error) {
	path, err := agentStatePath(agentID)
	if err != nil {
		return AgentState{}, err
	}
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return AgentState{}, nil
		}
		return AgentState{}, err
	}
	var state AgentState
	if err := json.Unmarshal(data, &state); err != nil {
		return AgentState{}, err
	}
	return state, nil
}

// disciplineAnchorPath returns the full path to the per-agent
// discipline-anchors.md file. Lives alongside last_state.json under
// the same per-agent directory so a single agentStateDir lookup serves
// both R20 AgentState persistence and K-12 anchor-checksum verification.
func disciplineAnchorPath(agentID string) (string, error) {
	dir, err := agentStateDir(agentID)
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, "discipline-anchors.md"), nil
}

// ComputeDisciplineAnchorSHA256 reads the agent's discipline-anchors.md
// file and returns its SHA-256 hex digest. Returns ("", nil) if the file
// is absent (acceptable first-write state — caller decides whether to
// treat as drift via VerifyDisciplineAnchor's stored-vs-current logic).
// Returns ("", err) on I/O error.
//
// Phase K K-12 — feeds VerifyDisciplineAnchor's drift detection during
// R16 bootstrap.
func ComputeDisciplineAnchorSHA256(agentID string) (string, error) {
	path, err := disciplineAnchorPath(agentID)
	if err != nil {
		return "", err
	}
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return "", nil
		}
		return "", err
	}
	sum := sha256.Sum256(data)
	return hex.EncodeToString(sum[:]), nil
}

// VerifyDisciplineAnchor compares the stored AgentState's
// DisciplineAnchorSHA256 against the currently-computed SHA. Returns
// (matches, currentSHA, err).
//
// matches=false signals R16 bootstrap should halt + auto-PM peer per
// K-17 mutual-halt protocol (autocompact-induced anchor drift OR file
// accidentally deleted between checkpoints).
//
// Three semantic branches:
//  1. stored empty → matches=true (first-ever bootstrap; no drift
//     detectable yet; caller persists currentSHA so subsequent boots
//     can detect drift)
//  2. stored non-empty + file absent → matches=false (drift signal:
//     anchor file was present at last checkpoint but is now gone)
//  3. stored non-empty + file present → match by SHA equality
//
// Phase K K-12.
func VerifyDisciplineAnchor(agentID string, stored AgentState) (bool, string, error) {
	current, err := ComputeDisciplineAnchorSHA256(agentID)
	if err != nil {
		return false, "", err
	}
	if stored.DisciplineAnchorSHA256 == "" {
		return true, current, nil
	}
	if current == "" {
		// File absent but stored non-empty — anchor file vanished
		// between last checkpoint and this bootstrap. Drift signal.
		return false, "", nil
	}
	if current != stored.DisciplineAnchorSHA256 {
		return false, current, nil
	}
	return true, current, nil
}
