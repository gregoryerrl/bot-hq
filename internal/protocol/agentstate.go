package protocol

import (
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
	// ~/.bot-hq/phase/<active>.md presence at bootstrap.
	LastPhaseDoc string `json:"last_phase_doc"`

	// LastRatchetPull is when the agent last read
	// ~/.bot-hq/ratchets/active.md.
	LastRatchetPull time.Time `json:"last_ratchet_pull_at"`

	// LastStateWrite is when this AgentState was written. Stale state
	// (>1h delta from now()) suggests skipped writes; agent flags
	// missing-write-discipline at bootstrap.
	LastStateWrite time.Time `json:"last_state_write_at"`
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
