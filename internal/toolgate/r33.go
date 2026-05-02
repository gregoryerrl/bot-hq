package toolgate

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strconv"

	"github.com/gregoryerrl/bot-hq/internal/hub"
)

// ChecklistClass identifies which gate-file class a verification targets.
// Per Phase L L-5 R33 PRE-EXECUTE-GATE-FILE-READ: HANDS-class execute
// actions consult the corresponding gate-file at ~/.bot-hq/gates/.
type ChecklistClass string

const (
	ClassCommit ChecklistClass = "commit"
	ClassPush   ChecklistClass = "push"
	ClassMerge  ChecklistClass = "merge"
)

// gateFileName returns the canonical gate-file basename for the class.
func gateFileName(class ChecklistClass) string {
	switch class {
	case ClassCommit:
		return "pre-commit-checklist.md"
	case ClassPush:
		return "pre-push-checklist.md"
	case ClassMerge:
		return "pre-merge-checklist.md"
	}
	return ""
}

// gateFilePath returns the absolute path to the gate-file for the class.
// Honors BOT_HQ_HOME for tests; defaults to ~/.bot-hq/gates/<file>.
func gateFilePath(class ChecklistClass) string {
	if home := os.Getenv("BOT_HQ_HOME"); home != "" {
		return filepath.Join(home, "gates", gateFileName(class))
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return ""
	}
	return filepath.Join(home, ".bot-hq", "gates", gateFileName(class))
}

// agentStatePath returns the absolute path to <agent>/last_state.json.
// Honors BOT_HQ_HOME for tests; defaults to ~/.bot-hq/<agent>/last_state.json.
func agentStatePath(agentID string) string {
	if home := os.Getenv("BOT_HQ_HOME"); home != "" {
		return filepath.Join(home, agentID, "last_state.json")
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return ""
	}
	return filepath.Join(home, ".bot-hq", agentID, "last_state.json")
}

// defaultChecklistFreshnessWindow is the msg-count window for AgentState
// SHA-cite freshness. Per F4-unification: AgentState cite must be within
// 5 self-agent messages of the execute-fire turn.
const defaultChecklistFreshnessWindow = 5

// checklistFreshnessWindow honors BRIAN_CHECKLIST_FRESHNESS_WINDOW for
// L-6 retro tunability.
func checklistFreshnessWindow() int {
	if v := os.Getenv("BRIAN_CHECKLIST_FRESHNESS_WINDOW"); v != "" {
		if n, err := strconv.Atoi(v); err == nil && n > 0 {
			return n
		}
	}
	return defaultChecklistFreshnessWindow
}

// commitOverrideEnvVar is the only honored override per F9 amend +
// gate-files source-of-truth. Push has NO normal-bypass (force-push uses
// R29 elevated gate, separate path); merge is USER-ONLY ABSOLUTE per R12
// GATE-PROTOCOL — no agent-side override.
const commitOverrideEnvVar = "BRIAN_PRE_COMMIT_GATE_OVERRIDE"

// Footer regexes for SHA-cite lookup in commit messages.
// SHA must be exactly 64 hex chars (sha256). Case-insensitive prefix.
var (
	preCommitChecklistShaRegex = regexp.MustCompile(`(?im)^pre-commit-checklist-sha:\s*([0-9a-fA-F]{64})\s*$`)
	prePushChecklistShaRegex   = regexp.MustCompile(`(?im)^pre-push-checklist-sha:\s*([0-9a-fA-F]{64})\s*$`)
)

// ChecklistVerdict reports the outcome of an R33 pre-execute gate-CHECK.
type ChecklistVerdict struct {
	// Allow is true if the execute action may proceed.
	Allow bool

	// Reason is a human-readable explanation (filled on Allow=false; may
	// also fill on Allow=true with "skipped: <form>" for soft-allow paths).
	Reason string

	// SkippedForm is non-empty if verification was skipped due to soft-allow
	// path (override / gatefile-absent / hubdb-absent / etc.).
	SkippedForm string
}

// VerifyChecklistCite runs R33 pre-execute verification for the given
// gate-class. Decision tree:
//
//  1. Commit-class override env var (BRIAN_PRE_COMMIT_GATE_OVERRIDE=1) →
//     Allow=true SkippedForm="override". Push/merge have NO normal-bypass.
//  2. Compute current SHA256 of gate-file at ~/.bot-hq/gates/<file>.
//     Absent → Allow=true SkippedForm="gatefile-absent" (fail-soft per
//     K-13 pattern; non-trio Claude Code instances should not block).
//  3. For commit/push classes: extract footer SHA-cite from commit
//     message. Match → Allow=true. Mismatch or absent → fall through.
//  4. AgentState path (mandatory for merge; fallback for commit/push):
//     read ~/.bot-hq/<agent>/last_state.json; verify
//     pre_<class>_checklist_sha_seen matches current SHA AND
//     pre_<class>_checklist_sha_seen_at_msg_id is within freshness window
//     (default 5 self-agent messages) of the latest self-agent msg-id
//     queried from hub.db.
//  5. All paths fail → Allow=false with descriptive Reason.
//
// commitMsg is only consulted for commit/push classes; ignored for merge.
// agentID resolves AgentState path + hub.db self-msg-id query.
//
// Phase L L-5 commit-2.
func VerifyChecklistCite(class ChecklistClass, agentID, commitMsg string) ChecklistVerdict {
	// (1) Override (commit-class only per F9)
	if class == ClassCommit && os.Getenv(commitOverrideEnvVar) == "1" {
		return ChecklistVerdict{Allow: true, SkippedForm: "override"}
	}

	// (2) Compute current gate-file SHA
	gatePath := gateFilePath(class)
	if gatePath == "" {
		return ChecklistVerdict{Allow: true, SkippedForm: "homedir-resolve-error"}
	}
	gateContent, err := os.ReadFile(gatePath)
	if err != nil {
		if os.IsNotExist(err) {
			return ChecklistVerdict{Allow: true, SkippedForm: "gatefile-absent"}
		}
		return ChecklistVerdict{Allow: true, SkippedForm: "gatefile-read-error"}
	}
	currentSHA := sha256Hex(gateContent)

	// (3) Footer SHA-cite path (commit/push only)
	if class == ClassCommit || class == ClassPush {
		if footerSHAMatchesCurrent(class, commitMsg, currentSHA) {
			return ChecklistVerdict{Allow: true}
		}
	}

	// (4) AgentState SHA-cite path
	verdict := verifyAgentStateCite(class, agentID, currentSHA)
	return verdict
}

// footerSHAMatchesCurrent checks if the commit message contains the
// expected Pre-<class>-checklist-SHA footer matching currentSHA.
func footerSHAMatchesCurrent(class ChecklistClass, commitMsg, currentSHA string) bool {
	var rx *regexp.Regexp
	switch class {
	case ClassCommit:
		rx = preCommitChecklistShaRegex
	case ClassPush:
		rx = prePushChecklistShaRegex
	default:
		return false
	}
	matches := rx.FindStringSubmatch(commitMsg)
	if matches == nil {
		return false
	}
	return matches[1] == currentSHA
}

// verifyAgentStateCite reads <agent>/last_state.json and verifies the
// pre_<class>_checklist_sha_seen field matches currentSHA AND its
// _at_msg_id companion is within freshness window of latest self-msg-id
// in hub.db.
func verifyAgentStateCite(class ChecklistClass, agentID, currentSHA string) ChecklistVerdict {
	statePath := agentStatePath(agentID)
	stateBytes, err := os.ReadFile(statePath)
	if err != nil {
		if os.IsNotExist(err) {
			return ChecklistVerdict{
				Allow:  false,
				Reason: fmt.Sprintf("AgentState %s not found; cannot verify pre_%s_checklist_sha_seen field per R33 PRE-EXECUTE-GATE-FILE-READ", statePath, class),
			}
		}
		return ChecklistVerdict{Allow: true, SkippedForm: "agentstate-read-error"}
	}
	state, err := parseAgentState(stateBytes)
	if err != nil {
		return ChecklistVerdict{Allow: true, SkippedForm: "agentstate-parse-error"}
	}

	shaField := fmt.Sprintf("pre_%s_checklist_sha_seen", class)
	atMsgIDField := fmt.Sprintf("pre_%s_checklist_sha_seen_at_msg_id", class)

	seenSHA, _ := state[shaField].(string)
	if seenSHA == "" {
		return ChecklistVerdict{
			Allow:  false,
			Reason: fmt.Sprintf("AgentState field %s missing or empty; consult gate-file %s + refresh AgentState before %s-fire", shaField, gateFileName(class), class),
		}
	}
	if seenSHA != currentSHA {
		return ChecklistVerdict{
			Allow:  false,
			Reason: fmt.Sprintf("AgentState %s does not match current %s SHA (seen=%q current=%q); re-consult gate-file + refresh AgentState", shaField, gateFileName(class), seenSHA, currentSHA),
		}
	}

	atMsgIDFloat, _ := state[atMsgIDField].(float64)
	atMsgID := int64(atMsgIDFloat)
	if atMsgID <= 0 {
		return ChecklistVerdict{
			Allow:  false,
			Reason: fmt.Sprintf("AgentState field %s missing or non-positive; refresh AgentState before %s-fire", atMsgIDField, class),
		}
	}

	// Freshness check via hub.db latest self-msg-id
	dbPath := hubDBPath()
	if dbPath == "" {
		return ChecklistVerdict{Allow: true, SkippedForm: "homedir-resolve-error"}
	}
	if _, err := os.Stat(dbPath); err != nil {
		if os.IsNotExist(err) {
			return ChecklistVerdict{Allow: true, SkippedForm: "hubdb-absent"}
		}
		return ChecklistVerdict{Allow: true, SkippedForm: "hubdb-stat-error"}
	}
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		return ChecklistVerdict{Allow: true, SkippedForm: "hubdb-open-error"}
	}
	defer db.Close()

	latest, found, err := db.GetLatestMessageFrom(agentID)
	if err != nil {
		return ChecklistVerdict{Allow: true, SkippedForm: "hubdb-query-error"}
	}
	if !found {
		// No messages from this agent — accept AgentState cite as authoritative
		// (test/fresh-install case; freshness check requires at-least-one msg).
		return ChecklistVerdict{Allow: true, SkippedForm: "no-self-messages"}
	}

	window := int64(checklistFreshnessWindow())
	delta := latest.ID - atMsgID
	if delta > window {
		return ChecklistVerdict{
			Allow: false,
			Reason: fmt.Sprintf(
				"AgentState %s is stale: at_msg_id=%d but current_max_self_msg_id=%d (delta=%d > window=%d self-agent messages); re-consult gate-file + refresh AgentState before %s-fire",
				atMsgIDField, atMsgID, latest.ID, delta, window, class,
			),
		}
	}

	return ChecklistVerdict{Allow: true}
}

// sha256Hex returns the hex-encoded SHA256 of data.
func sha256Hex(data []byte) string {
	sum := sha256.Sum256(data)
	return hex.EncodeToString(sum[:])
}

// parseAgentState parses last_state.json into a flat field-lookup map.
// Handles both top-level fields and fields nested under
// session_state_at_save (where pre_<class>_checklist_sha_seen lives in
// the canonical AgentState shape).
func parseAgentState(data []byte) (map[string]interface{}, error) {
	var raw map[string]interface{}
	if err := json.Unmarshal(data, &raw); err != nil {
		return nil, err
	}
	out := map[string]interface{}{}
	for k, v := range raw {
		out[k] = v
	}
	if inner, ok := raw["session_state_at_save"].(map[string]interface{}); ok {
		for k, v := range inner {
			if _, exists := out[k]; !exists {
				out[k] = v
			}
		}
	}
	return out, nil
}
