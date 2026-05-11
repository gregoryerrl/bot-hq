package toolgate

import (
	"encoding/json"
	"fmt"
	"io"
	"os"
)

// HookInput is the JSON shape Claude Code passes via stdin to PreToolUse
// hooks. We only need ToolName + ToolInput.command for the K-16 gate;
// other fields are tolerated for forward compatibility.
//
// Reference: Claude Code hooks doc (PreToolUse event payload).
type HookInput struct {
	SessionID     string         `json:"session_id,omitempty"`
	ToolName      string         `json:"tool_name"`
	ToolInput     map[string]any `json:"tool_input,omitempty"`
	HookEventName string         `json:"hook_event_name,omitempty"`
}

// ExitAllow indicates the tool call is permitted to proceed.
const ExitAllow = 0

// ExitBlock indicates Claude Code should block the tool call. Per Claude
// Code hook protocol, exit code 2 + stderr message blocks the tool call
// and surfaces the stderr text to the user.
const ExitBlock = 2

// RunHook is the PreToolUse hook entry point. Reads the hook input JSON
// from stdin, applies the K-16 class-split gate logic, and returns an
// exit code (0=allow / 2=block).
//
// Behavior:
//  1. Decode stdin JSON; on parse error → allow (defensive — don't block
//     agent on hook-side bugs).
//  2. If ToolName != "Bash" → allow (MVP scope is Bash-only per Rain
//     msg 6411; Edit/Write deferred).
//  3. Extract command string from ToolInput["command"]; non-string or
//     missing → allow (defensive).
//  4. Read BOT_HQ_AGENT_ID env var; missing → allow with no warning
//     (defensive default — don't block non-duo Claude Code instances).
//  5. If agent_id == "rain" AND IsHANDSExecutePattern(command) →
//     write block message to stderr + return ExitBlock.
//  6. Otherwise → allow.
//
// Stderr message includes the matched pattern + recovery anchor pointer
// + recovery action (per Rain msg 6418 refinement).
//
// Phase K K-16.
func RunHook(stdin io.Reader, stderr io.Writer) int {
	var input HookInput
	if err := json.NewDecoder(stdin).Decode(&input); err != nil {
		return ExitAllow
	}

	// Phase R R2 hub_broadcast Rain-gate (literal tool-name match per
	// Refine-5; agent-id env-var check). Fires BEFORE Bash early-return
	// so non-Bash MCP tool invocation reaches the gate.
	if input.ToolName == HubBroadcastToolName {
		agentID := os.Getenv("BOT_HQ_AGENT_ID")
		if allow, reason := VerifyHubBroadcastRainGate(agentID); !allow {
			fmt.Fprint(stderr, reason)
			return ExitBlock
		}
		return ExitAllow
	}

	if input.ToolName != "Bash" {
		return ExitAllow
	}

	cmd, _ := input.ToolInput["command"].(string)
	if cmd == "" {
		return ExitAllow
	}

	agentID := os.Getenv("BOT_HQ_AGENT_ID")
	if agentID == "" {
		return ExitAllow
	}

	// Rain-only branch: K-16 class-split gate (block HANDS-execute) +
	// historical K-13 R12 commit-gate path (preserved for rain commit
	// case; brian K-13 enforcement happens via VerifyCommit direct
	// invocation elsewhere — see r12_test.go for direct-call coverage).
	if agentID == "rain" {
		if IsHANDSExecutePattern(cmd) {
			tokens := tokenize(cmd)
			pattern := ""
			if len(tokens) >= 2 {
				pattern = tokens[0] + " " + tokens[1]
			}
			fmt.Fprintf(stderr,
				"K-16 class-split gate: rain (EYES) cannot fire HANDS execute pattern.\n"+
					"Command: %s\n"+
					"Pattern: %s\n"+
					"Re-anchor: ~/.bot-hq/rain/discipline-anchors.md § class-split\n"+
					"Brian (HANDS) executes; Rain drafts + surfaces + greenflags.\n"+
					"Recovery: if user authorized this directly, brian fires; if PM-from-brian implied this, hold for user broadcast.\n",
				cmd, pattern,
			)
			return ExitBlock
		}
		if IsCommitPattern(cmd) {
			verdict := VerifyCommit(cmd, agentID)
			if !verdict.Allow {
				fmt.Fprintf(stderr,
					"K-13 R12 pre-commit gate: commit blocked.\n"+
						"Reason: %s\n"+
						"Re-anchor: ~/.bot-hq/%s/discipline-anchors.md § R12 BRAIN-2nd pre-commit\n"+
						"Recovery: surface diff to peer for BRAIN-2nd review; on peer greenflag (substring 'BRAIN-AGREED' or 'GREENFLAG' in their reply), add `peer-greenflag-msg-id: <N>` footer to commit message.\n"+
						"Bypass (emergency only, logged): export BRIAN_R12_OVERRIDE=1 before commit.\n",
					verdict.Reason, agentID,
				)
				return ExitBlock
			}
		}
		return ExitAllow
	}

	// L-5 R33 pre-commit-checklist gate-CHECK (HANDS-class agents,
	// typically brian). Fires on git commit; commit-message footer must
	// include Pre-commit-checklist-SHA: <sha256> matching current
	// gate-file SHA, OR AgentState pre_commit_checklist_sha_seen field
	// current within freshness window.
	if IsCommitPattern(cmd) {
		commitMsg, _, _ := extractCommitMessage(cmd)
		r33 := VerifyChecklistCite(ClassCommit, agentID, commitMsg)
		if !r33.Allow {
			fmt.Fprintf(stderr,
				"L-5 R33 pre-commit-checklist gate-CHECK: commit blocked.\n"+
					"Reason: %s\n"+
					"Re-anchor: ~/.bot-hq/gates/pre-commit-checklist.md § Checklist (item-9 SHA-cite footer)\n"+
					"Recovery: consult pre-commit-checklist.md (all 9 items) before commit-fire; add `Pre-commit-checklist-SHA: <sha256>` footer to commit message OR refresh AgentState pre_commit_checklist_sha_seen field within last %d self-agent messages.\n"+
					"Bypass (emergency only, logged): export BRIAN_PRE_COMMIT_GATE_OVERRIDE=1 before commit.\n",
				r33.Reason, checklistFreshnessWindow(),
			)
			return ExitBlock
		}
	}

	// L-5 R33 pre-push-checklist gate-CHECK (no command-message inspection
	// for push class in MVP; AgentState path is sufficient first cut).
	// Push has NO normal-bypass per gate-file source-of-truth; force-push
	// uses R29 elevated gate (separate path; both gates fire independently).
	if IsPushPattern(cmd) {
		r33 := VerifyChecklistCite(ClassPush, agentID, "")
		if !r33.Allow {
			fmt.Fprintf(stderr,
				"L-5 R33 pre-push-checklist gate-CHECK: push blocked.\n"+
					"Reason: %s\n"+
					"Re-anchor: ~/.bot-hq/gates/pre-push-checklist.md § Checklist (item-9 SHA-cite or AgentState path)\n"+
					"Recovery: consult pre-push-checklist.md (all 9 items) before push-fire; refresh AgentState pre_push_checklist_sha_seen field within last %d self-agent messages of push-fire turn.\n"+
					"NO normal-bypass for push class; force-push uses R29 elevated gate (separate path).\n",
				r33.Reason, checklistFreshnessWindow(),
			)
			return ExitBlock
		}
	}

	// L-5 R33 pre-merge-checklist gate-CHECK. Merge is USER-ONLY ABSOLUTE
	// per R12 GATE-PROTOCOL — no agent-side override. Gate-file
	// consultation is pre-fire discipline mechanism, NOT substitute for
	// user-verbatim merge-token authorization.
	if IsMergePattern(cmd) {
		r33 := VerifyChecklistCite(ClassMerge, agentID, "")
		if !r33.Allow {
			fmt.Fprintf(stderr,
				"L-5 R33 pre-merge-checklist gate-CHECK: merge blocked.\n"+
					"Reason: %s\n"+
					"Re-anchor: ~/.bot-hq/gates/pre-merge-checklist.md § Checklist (item-8 AgentState SHA-cite)\n"+
					"Recovery: consult pre-merge-checklist.md (all 8 items) before merge-fire; refresh AgentState pre_merge_checklist_sha_seen field within last %d self-agent messages of merge-fire turn.\n"+
					"NO agent-side override for merge class; user-verbatim merge-token required per R12 USER-ONLY-ABSOLUTE.\n",
				r33.Reason, checklistFreshnessWindow(),
			)
			return ExitBlock
		}
	}

	return ExitAllow
}
