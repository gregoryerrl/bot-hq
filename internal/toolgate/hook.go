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
//     (defensive default — don't block non-trio Claude Code instances).
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

	if agentID != "rain" {
		// HANDS class (brian) or other agent — allow.
		return ExitAllow
	}

	if !IsHANDSExecutePattern(cmd) {
		return ExitAllow
	}

	// rain + HANDS execute pattern → block.
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
