package protocol

import (
	"os"
	"path/filepath"
)

// AgentStatePath returns the per-session per-agent state.json path under
// the Z-3 sessions-as-containers substrate:
//
//	<canonRoot>/sessions/<sessionID>/<agent>/state.json
//
// This is the SINGLE-PATH helper for BRAIN-duo agents (brian, rain) whose
// state lives inside the containing session. emma is stateless (no state
// file at all); callers must not invoke this for emma. Clive (voice) keeps
// its top-level state at ~/.bot-hq/clive/last_state.json via the legacy
// agentStateDir/agentStatePath helpers — clive is not a per-session agent.
//
// canonRoot is the canonical-store root (~/.bot-hq/ by default; honors
// BOT_HQ_HOME when set). sessionID is the scope-keyed slug-uuid identifier
// allocated by hub_session_open. agent is the agent-id (brian or rain).
//
// Returns an empty string if any arg is empty — callers are expected to
// have a valid session-bound triple at the point of use.
func AgentStatePath(canonRoot, sessionID, agent string) string {
	if canonRoot == "" || sessionID == "" || agent == "" {
		return ""
	}
	return filepath.Join(canonRoot, "sessions", sessionID, agent, "state.json")
}

// CanonRoot returns the canonical-store root respecting BOT_HQ_HOME. Mirrors
// agentStateDir's resolution path. Returns "" + error from os.UserHomeDir on
// resolution failure.
func CanonRoot() (string, error) {
	if home := os.Getenv("BOT_HQ_HOME"); home != "" {
		return home, nil
	}
	homeDir, err := os.UserHomeDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(homeDir, ".bot-hq"), nil
}
