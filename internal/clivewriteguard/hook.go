// Package clivewriteguard implements the Phase P P-4b PreToolUse hook
// that enforces canonical-store-write-API-only authority on Clive per
// PhaseNv3CliveExpansion (internal/protocol/disc.go:543) +
// phase-n.md:545.
//
// Enforcement model (BLOCKING — distinct from voicemirror's alert-only
// discipline):
//
//   - When BOT_HQ_AGENT_ID != "clive", hook is a no-op (allow). Brian
//     and Rain need full Edit/Write/Bash authority for orchestration
//     and review work.
//   - When BOT_HQ_AGENT_ID == "clive", the following tools are BLOCKED:
//       Edit, Write, MultiEdit, NotebookEdit  → use POST /api/files/{path}/clive
//       Bash                                  → no shell; use HTTP API + hub_send
//   - All other tools (Read, Grep, Glob, hub_send variants, etc.)
//     remain allowed so Clive can investigate + coordinate freely.
//
// The block mechanism follows Claude Code hooks docs: exit code 2 +
// reason on stderr halts the tool call and surfaces the message to
// Claude. PreToolUse hook is the right surface (not Stop / SessionEnd)
// because we want to prevent the action, not log after-the-fact.
//
// Per Phase P scope-lock §P-4b + Rain BRAIN-2nd P-4-bundle-vs-split
// lean: this is the defense-in-depth sibling to P-4a (prompt-build).
// P-4a declares the constraint in Clive's prompt; P-4b enforces it
// at runtime so prompt-drift can't open a write-bypass class.
package clivewriteguard

import (
	"encoding/json"
	"fmt"
	"io"
	"os"
	"strings"
)

// HookInput is the JSON shape Claude Code passes via stdin to PreToolUse
// hooks. We need ToolName to dispatch; ToolInput is captured for
// reason-text formatting (e.g., echoing the attempted file_path).
type HookInput struct {
	ToolName      string         `json:"tool_name"`
	ToolInput     map[string]any `json:"tool_input,omitempty"`
	HookEventName string         `json:"hook_event_name,omitempty"`
}

// Exit codes follow Claude Code hooks convention.
const (
	ExitAllow = 0 // success / allow tool call
	ExitBlock = 2 // block + surface stderr to Claude
)

const agentIDEnvVar = "BOT_HQ_AGENT_ID"

// blockedTools is the set of tool names denied to Clive. Lowercase
// comparison so casing variations don't slip through.
var blockedTools = map[string]string{
	"edit":         "Clive must use POST /api/files/{path}/clive — direct Edit is bare-filesystem-write (forbidden by PhaseNv3CliveExpansion canonical-store-write-API-only authority).",
	"write":        "Clive must use POST /api/files/{path}/clive — direct Write is bare-filesystem-write (forbidden by PhaseNv3CliveExpansion canonical-store-write-API-only authority).",
	"multiedit":    "Clive must use POST /api/files/{path}/clive — direct MultiEdit is bare-filesystem-write (forbidden by PhaseNv3CliveExpansion canonical-store-write-API-only authority).",
	"notebookedit": "Clive must use POST /api/files/{path}/clive — direct NotebookEdit is bare-filesystem-write (forbidden by PhaseNv3CliveExpansion canonical-store-write-API-only authority).",
	"bash":         "Clive cannot use Bash. Writes go through POST /api/files/{path}/clive; messages go through mcp__bot-hq__hub_send. Bash bypasses the canonical-store-write-API-only authority enforced by PhaseNv3CliveExpansion.",
}

// RunHook is the PreToolUse entry point. Reads HookInput from stdin,
// inspects BOT_HQ_AGENT_ID + ToolName, returns the appropriate exit
// code (writing the reason to stderr on block).
//
// Returns the exit code so callers (cmd binary + tests) can propagate
// to os.Exit. Errors during JSON decode default to allow (don't break
// the agent on malformed harness input — log to stderr for diagnosis).
func RunHook(stdin io.Reader, stderr io.Writer) int {
	agentID := strings.ToLower(strings.TrimSpace(os.Getenv(agentIDEnvVar)))
	if agentID != "clive" {
		// No-op for Brian / Rain / unset — they keep full tool access.
		return ExitAllow
	}
	var input HookInput
	dec := json.NewDecoder(stdin)
	if err := dec.Decode(&input); err != nil {
		// Malformed input: don't break the agent — log + allow.
		fmt.Fprintf(stderr, "clivewriteguard: decode error: %v (allowing tool call)\n", err)
		return ExitAllow
	}
	toolName := strings.ToLower(strings.TrimSpace(input.ToolName))
	reason, blocked := blockedTools[toolName]
	if !blocked {
		return ExitAllow
	}
	// Block: emit reason to stderr (surfaces to Claude per hook docs)
	// + include attempted path / command if present for debugging.
	fmt.Fprintln(stderr, reason)
	if path, ok := stringField(input.ToolInput, "file_path"); ok {
		fmt.Fprintf(stderr, "Attempted path: %s\n", path)
	}
	if cmd, ok := stringField(input.ToolInput, "command"); ok {
		fmt.Fprintf(stderr, "Attempted command: %s\n", truncate(cmd, 200))
	}
	return ExitBlock
}

// stringField extracts a string-typed field from the ToolInput map.
// Returns ("", false) when missing or wrong-typed.
func stringField(m map[string]any, key string) (string, bool) {
	if m == nil {
		return "", false
	}
	v, ok := m[key]
	if !ok {
		return "", false
	}
	s, ok := v.(string)
	return s, ok
}

// truncate clamps a string to maxLen chars with an ellipsis marker
// when truncated. Avoids dumping multi-KB Bash heredocs into stderr.
func truncate(s string, maxLen int) string {
	if len(s) <= maxLen {
		return s
	}
	return s[:maxLen] + "…"
}
