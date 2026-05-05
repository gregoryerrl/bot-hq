// Package voicemirror implements the Phase N v2 #3 N-2 PreToolUse hook
// that observes Write tool calls against user-artifact path patterns
// per R40 VOICE-MIRROR-DISCIPLINE. Hook is alert-only (NOT blocking) —
// matched paths produce a discipline-record entry at
// ~/.bot-hq/voice-mirror-log.md for retro review at phase-close.
//
// INCLUDE patterns (RATIFIED at scope-lock v2 OQ-1b PASS-3 per Brian
// msg 8103 + Rain msg 8102):
//   - ~/Documents/* (user document area)
//   - ~/Desktop/* (user desktop area)
//   - ~/.bot-hq/projects/<project>/{plans,eod,clips}/* (LOCAL planning
//     artifacts not git-tracked; user-facing artifact subclasses)
//   - <any-path>/CLAUDE.md (project-root user-authored config)
//   - <any-path>/README.md (project-root user-facing doc)
//
// SKIP patterns (override INCLUDE):
//   - **/memory/** (auto-memory writes are agent-internal anchors per
//     Rain push-back c — not user-voice mirroring class)
//   - **/.git/**, **/.cache/**, **/node_modules/**
//
// DEFER (Phase N v3 / Tier-2 per Rain push-back a): dynamic path
// extraction from user-message regex (impl-heavy + fuzzy semantics +
// DB-dependency at hook-time; MVP discipline = static set only).
package voicemirror

import (
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"time"
)

// HookInput is the JSON shape Claude Code passes via stdin to PreToolUse
// hooks. We only need ToolName + ToolInput.file_path + ToolInput.content;
// other fields are tolerated for forward compatibility.
type HookInput struct {
	SessionID     string         `json:"session_id,omitempty"`
	ToolName      string         `json:"tool_name"`
	ToolInput     map[string]any `json:"tool_input,omitempty"`
	HookEventName string         `json:"hook_event_name,omitempty"`
}

// ExitAllow is the only exit code this hook returns — voice-mirror
// discipline is alert-only per scope-lock v2 §Acceptance #3 (consistent
// with R6 OUTBOUND-DISCIPLINE detect-only origin).
const ExitAllow = 0

const (
	agentIDEnvVar = "BOT_HQ_AGENT_ID"
	logPathEnvVar = "BOT_HQ_VOICE_MIRROR_LOG_PATH"
	snippetMaxLen = 200
)

// RunHook is the PreToolUse hook entry point for voice-mirror discipline.
// Always returns ExitAllow (alert-only); on user-artifact path match,
// appends a log entry to the voice-mirror log file (default
// ~/.bot-hq/voice-mirror-log.md; overridable via BOT_HQ_VOICE_MIRROR_LOG_PATH
// env for tests).
func RunHook(stdin io.Reader, _ io.Writer) int {
	var input HookInput
	if err := json.NewDecoder(stdin).Decode(&input); err != nil {
		return ExitAllow
	}
	if input.ToolName != "Write" {
		return ExitAllow
	}
	filePath, _ := input.ToolInput["file_path"].(string)
	if filePath == "" {
		return ExitAllow
	}
	if !MatchesUserArtifactPath(filePath) {
		return ExitAllow
	}

	agentID := os.Getenv(agentIDEnvVar)
	if agentID == "" {
		agentID = "unknown"
	}
	content, _ := input.ToolInput["content"].(string)
	snippet := truncateSnippet(content)

	logPath := os.Getenv(logPathEnvVar)
	if logPath == "" {
		home, _ := os.UserHomeDir()
		logPath = filepath.Join(home, ".bot-hq", "voice-mirror-log.md")
	}

	appendLogEntry(logPath, agentID, filePath, snippet)
	return ExitAllow
}

// MatchesUserArtifactPath returns true if the given path matches any
// INCLUDE pattern AND no SKIP pattern. Exported for hook-external
// path-set audits.
func MatchesUserArtifactPath(path string) bool {
	if matchesSkip(path) {
		return false
	}
	return matchesInclude(path)
}

func matchesSkip(path string) bool {
	if strings.Contains(path, "/memory/") || strings.HasSuffix(path, "/memory") {
		return true
	}
	if strings.Contains(path, "/.git/") {
		return true
	}
	if strings.Contains(path, "/.cache/") {
		return true
	}
	if strings.Contains(path, "/node_modules/") {
		return true
	}
	return false
}

func matchesInclude(path string) bool {
	home, _ := os.UserHomeDir()

	if home != "" {
		if strings.HasPrefix(path, filepath.Join(home, "Documents")+string(filepath.Separator)) {
			return true
		}
		if strings.HasPrefix(path, filepath.Join(home, "Desktop")+string(filepath.Separator)) {
			return true
		}
		prefix := filepath.Join(home, ".bot-hq", "projects") + string(filepath.Separator)
		if strings.HasPrefix(path, prefix) {
			rest := strings.TrimPrefix(path, prefix)
			parts := strings.SplitN(rest, string(filepath.Separator), 3)
			if len(parts) >= 2 {
				class := parts[1]
				if class == "plans" || class == "eod" || class == "clips" {
					return true
				}
			}
		}
	}

	base := filepath.Base(path)
	if base == "CLAUDE.md" || base == "README.md" {
		return true
	}

	return false
}

func truncateSnippet(content string) string {
	s := content
	if len(s) > snippetMaxLen {
		s = s[:snippetMaxLen] + "…"
	}
	return strings.ReplaceAll(s, "\n", " ")
}

func appendLogEntry(logPath, agentID, filePath, snippet string) {
	entry := fmt.Sprintf("- %s | %s | %s | %q\n",
		time.Now().UTC().Format(time.RFC3339),
		agentID,
		filePath,
		snippet,
	)
	f, err := os.OpenFile(logPath, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o644)
	if err != nil {
		return
	}
	defer f.Close()
	_, _ = f.WriteString(entry)
}
