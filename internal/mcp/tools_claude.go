package mcp

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"strings"
	"time"

	"github.com/google/uuid"
	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
	tmuxpkg "github.com/gregoryerrl/bot-hq/internal/tmux"
	"github.com/mark3labs/mcp-go/mcp"
)

// --- Claude Code tools ---

func claudeList(db *hub.DB) ToolDef {
	tool := mcp.NewTool("claude_list",
		mcp.WithDescription("Discover and list all running Claude Code sessions in tmux. Returns session IDs, project paths, tmux targets, and status."),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		// Discover live sessions in tmux
		discovered, err := tmuxpkg.DiscoverClaudeSessions()
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("discovery failed: %v", err)), nil
		}

		// Sync with DB: register new ones, update existing
		var results []hub.ClaudeSession
		for _, d := range discovered {
			existing, err := db.FindClaudeSessionByTarget(d.TmuxTarget)
			if err == nil {
				// Update existing
				db.UpdateClaudeSessionStatus(existing.ID, "running", "")
				existing.Status = "running"
				existing.PID = d.PID
				results = append(results, existing)
			} else {
				// New session found — register as attached
				id := uuid.New().String()[:8]
				sess := hub.ClaudeSession{
					ID:         id,
					Project:    d.CWD,
					TmuxTarget: d.TmuxTarget,
					PID:        d.PID,
					Mode:       "attached",
					Status:     "running",
					Started:    time.Now(),
				}
				db.InsertClaudeSession(sess)
				results = append(results, sess)
			}
		}

		// Also include DB sessions that weren't found (mark as stopped)
		dbSessions, _ := db.ListClaudeSessions("running")
		for _, s := range dbSessions {
			found := false
			for _, d := range discovered {
				if d.TmuxTarget == s.TmuxTarget {
					found = true
					break
				}
			}
			if !found {
				markSessionStoppedAndAgentOffline(db, s.ID)
				s.Status = "stopped"
			}
			// Check if already in results
			inResults := false
			for _, r := range results {
				if r.ID == s.ID {
					inResults = true
					break
				}
			}
			if !inResults {
				results = append(results, s)
			}
		}

		return mcp.NewToolResultText(toJSON(results)), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

func claudeRead(db *hub.DB) ToolDef {
	tool := mcp.NewTool("claude_read",
		mcp.WithDescription("Read the latest output from a running Claude Code session. Captures the current tmux pane content."),
		mcp.WithString("session_id", mcp.Required(), mcp.Description("Session ID to read output from")),
		mcp.WithNumber("lines", mcp.Description("Number of lines to capture (default 50)")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		sessionID, err := req.RequireString("session_id")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		lines := req.GetInt("lines", 50)

		sess, err := db.GetClaudeSession(sessionID)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("session not found: %v", err)), nil
		}

		output, err := tmuxpkg.CapturePane(sess.TmuxTarget, lines)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("capture failed: %v", err)), nil
		}

		// Update last output in DB
		db.UpdateClaudeSessionStatus(sessionID, sess.Status, output)

		return mcp.NewToolResultText(output), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

func claudeMessage(db *hub.DB) ToolDef {
	tool := mcp.NewTool("claude_message",
		mcp.WithDescription("Send a message to a running Claude Code session. Detects if Claude is at prompt before sending. Returns captured output."),
		mcp.WithString("session_id", mcp.Required(), mcp.Description("Session ID to message")),
		mcp.WithString("message", mcp.Required(), mcp.Description("Message to send to Claude Code")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		sessionID, err := req.RequireString("session_id")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		message, err := req.RequireString("message")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		sess, err := db.GetClaudeSession(sessionID)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("session not found: %v", err)), nil
		}

		// Bug #3 fix: detect at-prompt via WaitForPrompt with a 750ms grace
		// window instead of the brittle last-line-of-10-lines heuristic. The
		// old heuristic checked if the literal last pane line ended in ❯/>
		// or was empty — which fails for Claude Code's actual rendering
		// because the visible ❯ is typically several lines above the literal
		// last line (input-box bottom rule + footer render below the prompt).
		// It also false-busy'd on transient mid-render frames during a
		// pane redraw. WaitForPrompt scans 30 lines for the byte anchor and
		// the 750ms grace tolerates partial-frame redraws.
		atPrompt, currentOutput, err := tmuxpkg.WaitForPrompt(sess.TmuxTarget, tmuxpkg.PromptCheckGrace)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("capture failed: %v", err)), nil
		}

		if !atPrompt {
			db.UpdateClaudeSessionStatus(sessionID, "busy", currentOutput)
			return mcp.NewToolResultText(fmt.Sprintf("[Claude is busy — not at prompt]\n%s", currentOutput)), nil
		}

		// Send the message
		if err := tmuxpkg.SendKeys(sess.TmuxTarget, message, true); err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("send failed: %v", err)), nil
		}

		// Wait for Claude to start processing, then capture
		time.Sleep(2 * time.Second)
		output, _ := tmuxpkg.CapturePane(sess.TmuxTarget, 50)
		db.UpdateClaudeSessionStatus(sessionID, "running", output)

		return mcp.NewToolResultText(output), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

func claudeSend() ToolDef {
	tool := mcp.NewTool("claude_send",
		mcp.WithDescription("Send a one-shot task to Claude Code using --print mode. Returns the full output. Good for quick questions that don't need a persistent session."),
		mcp.WithString("prompt", mcp.Required(), mcp.Description("Prompt to send to Claude Code")),
		mcp.WithString("cwd", mcp.Description("Working directory (defaults to home)")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		prompt, err := req.RequireString("prompt")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		cwd := req.GetString("cwd", "")
		if cwd == "" {
			cwd, _ = os.UserHomeDir()
		}

		cmd := exec.CommandContext(ctx, "claude", "--print", "-p", prompt)
		cmd.Dir = cwd
		out, err := cmd.Output()
		if err != nil {
			// Try to get partial output
			if exitErr, ok := err.(*exec.ExitError); ok && len(exitErr.Stderr) > 0 {
				return mcp.NewToolResultText(fmt.Sprintf("[partial output]\n%s\n[stderr: %s]",
					strings.TrimSpace(string(out)), strings.TrimSpace(string(exitErr.Stderr)))), nil
			}
			if len(out) > 0 {
				return mcp.NewToolResultText(fmt.Sprintf("[partial output]\n%s", strings.TrimSpace(string(out)))), nil
			}
			return mcp.NewToolResultError(fmt.Sprintf("claude --print failed: %v", err)), nil
		}

		output := strings.TrimSpace(string(out))
		if len(output) > 30000 {
			output = output[:30000] + "\n...[truncated]"
		}

		return mcp.NewToolResultText(output), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

func claudeResume() ToolDef {
	tool := mcp.NewTool("claude_resume",
		mcp.WithDescription("Resume the last Claude Code conversation using -c flag. Continues where a previous session left off."),
		mcp.WithString("prompt", mcp.Required(), mcp.Description("Prompt to continue the conversation with")),
		mcp.WithString("cwd", mcp.Description("Working directory (defaults to home)")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		prompt, err := req.RequireString("prompt")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		cwd := req.GetString("cwd", "")
		if cwd == "" {
			cwd, _ = os.UserHomeDir()
		}

		cmd := exec.CommandContext(ctx, "claude", "-c", "--print", "-p", prompt)
		cmd.Dir = cwd
		out, err := cmd.Output()
		if err != nil {
			if len(out) > 0 {
				return mcp.NewToolResultText(fmt.Sprintf("[partial output]\n%s", strings.TrimSpace(string(out)))), nil
			}
			return mcp.NewToolResultError(fmt.Sprintf("claude -c --print failed: %v", err)), nil
		}

		output := strings.TrimSpace(string(out))
		if len(output) > 30000 {
			output = output[:30000] + "\n...[truncated]"
		}

		return mcp.NewToolResultText(output), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

// markSessionStoppedAndAgentOffline writes the bug-#4 invariant: stopping a
// claude session also flips its paired agent row (same ID, registered by
// hubSpawn) to offline. Without the agent flip, killed coders accumulate as
// stale-online ghost rows in the agents table. Both claudeStop (explicit
// kill) and claudeList reconciliation (implicit cleanup when tmux vanishes)
// must use this — duplicating the two-call sequence inline is what missed
// the second surface during initial implementation.
func markSessionStoppedAndAgentOffline(db *hub.DB, sessionID string) {
	db.StopClaudeSession(sessionID)
	db.UpdateAgentStatus(sessionID, protocol.StatusOffline)
}

func claudeStop(db *hub.DB) ToolDef {
	tool := mcp.NewTool("claude_stop",
		mcp.WithDescription("Stop a running Claude Code session by killing its tmux session. This is destructive."),
		mcp.WithString("session_id", mcp.Required(), mcp.Description("Session ID to stop")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		sessionID, err := req.RequireString("session_id")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		sess, err := db.GetClaudeSession(sessionID)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("session not found: %v", err)), nil
		}

		// Kill the tmux session
		if err := tmuxpkg.KillSession(sess.TmuxTarget); err != nil {
			// Session might already be dead — mark as stopped anyway
		}

		markSessionStoppedAndAgentOffline(db, sessionID)

		return mcp.NewToolResultText(toJSON(map[string]string{
			"status":     "stopped",
			"session_id": sessionID,
		})), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}
