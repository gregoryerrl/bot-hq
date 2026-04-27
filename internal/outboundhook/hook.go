// Package outboundhook implements the slice-5 H-22-bis Stop hook that
// flags turns where a bot-hq agent emits assistant text without making a
// hub_send tool call ("OUTBOUND-MISS"). The pane→hub egress failure class
// is symmetric to the hub→pane delivery class (H-39/H-41) — both produce
// the user-facing "agent didn't reply" symptom from different mechanisms.
//
// RunHook is the Stop-hook entry point: reads Claude Code's hook input
// from stdin, parses the transcript, applies a three-clause filter
// (text>0 AND (keyword|len>200) AND no hub_send), checks an idempotency
// ledger to dedupe re-invocations, and on positive emits an alert via
// hub.DB.InsertMessage from the agent identified by BOT_HQ_AGENT_ID.
//
// InstallTrioHook installs the Stop hook into ~/.claude/settings.json
// for long-lived trio agents (brian/rain) that aren't spawned via
// hub_spawn. Idempotent + non-clobbering: existing unrelated hooks
// preserved, our hook appended only if not already present by command
// path.
package outboundhook

import (
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// HookInput is the JSON shape Claude Code passes via stdin to Stop hooks.
// We only require transcript_path; other fields are tolerated for forward
// compatibility.
type HookInput struct {
	SessionID      string `json:"session_id,omitempty"`
	TranscriptPath string `json:"transcript_path"`
	HookEventName  string `json:"hook_event_name,omitempty"`
}

// transcriptEvent is a single line from a Claude Code transcript JSONL.
// Fields not relevant to the OUTBOUND-MISS check are tolerated as unknown.
type transcriptEvent struct {
	Type      string          `json:"type"`
	Timestamp string          `json:"timestamp,omitempty"`
	Message   transcriptMsg   `json:"message,omitempty"`
}

type transcriptMsg struct {
	Role    string             `json:"role,omitempty"`
	Content []transcriptBlock  `json:"content,omitempty"`
}

type transcriptBlock struct {
	Type  string `json:"type"`
	Text  string `json:"text,omitempty"`
	Name  string `json:"name,omitempty"`
}

// planningKeywords are content tokens that strongly indicate a turn
// produced planning/discussion text intended for the hub. Match on any
// keyword fires the v1 filter even if the text is short. Curated
// conservatively — adding noisy tokens raises FP rate.
var planningKeywords = []string{
	"DRAFT", "BRAIN", "concur", "pushback", "push-back", "SNAP",
	"[Rain", "[Brian", "[HUB", "[PM:", "BACKFILL", "RETRACT",
	"greenlight", "greenflag",
}

// minTextLenForFlag is the LOC-equivalent fallback when no planning
// keyword is found. v1 conservative: 200 chars (~3-4 lines of prose) is
// substantive enough to warrant flagging if it left without a hub_send.
const minTextLenForFlag = 200

// hubSendToolPrefix matches all bot-hq MCP tool calls that emit to the
// hub: hub_send, hub_flag, hub_register, etc. Any of these counts as
// the agent having "spoken" on the hub this turn — no OUTBOUND-MISS.
const hubSendToolPrefix = "mcp__bot-hq__hub_"

// dedupWindow is how recent a same-key prior emit must be to suppress a
// duplicate. Covers harness-retry + replay-debug edge cases without
// stalling legitimate same-turn-resume scenarios.
const dedupWindow = 60 * time.Second

// agentIDEnvVar names the env var spawn-contracts must set to identify
// which bot-hq agent the hook is running for.
const agentIDEnvVar = "BOT_HQ_AGENT_ID"

// dbPathEnvVar names the env var that overrides the default hub DB path
// (~/.bot-hq/hub.db). Used by tests; production omits.
const dbPathEnvVar = "BOT_HQ_DB_PATH"

// turnSummary is the aggregated state of the last assistant turn — text
// content total, presence of any hub_send-class tool call, and the
// turn's wall-clock timestamp (used as the dedup key).
type turnSummary struct {
	TextLen   int
	TextSnip  string
	HubSent   bool
	Timestamp string
}

// RunHook is the Stop-hook entry point. Reads Claude Code's stdin JSON,
// parses the transcript, applies the three-clause filter, dedupes via
// ledger, and emits an OUTBOUND-MISS alert through hub.DB.InsertMessage.
// Best-effort: any error path returns nil (the hook must not block the
// agent's Stop event).
func RunHook(stdin io.Reader) error {
	var input HookInput
	if err := json.NewDecoder(stdin).Decode(&input); err != nil {
		return nil
	}
	if input.TranscriptPath == "" {
		return nil
	}
	agentID := os.Getenv(agentIDEnvVar)
	if agentID == "" {
		// Hook is installed but the env var isn't set — likely a non-bot-hq
		// claude session. Silent no-op rather than spamming alerts for every
		// claude pane on the system.
		return nil
	}

	summary, err := parseLastTurn(input.TranscriptPath)
	if err != nil {
		return nil
	}
	if !shouldFlag(summary) {
		return nil
	}

	if alreadyFlaggedRecently(input.TranscriptPath, summary.Timestamp, time.Now()) {
		return nil
	}

	if err := emitAlert(agentID, summary.Timestamp, summary.TextSnip); err != nil {
		return nil
	}

	recordDedup(input.TranscriptPath, summary.Timestamp, time.Now())
	return nil
}

// shouldFlag applies the three-clause filter:
//  1. assistant_text_total_len > 0 (excludes tool-only turns)
//  2. (planning keyword present OR text length > minTextLenForFlag)
//  3. no hub_send-class tool call this turn
func shouldFlag(s turnSummary) bool {
	if s.TextLen == 0 {
		return false
	}
	if s.HubSent {
		return false
	}
	if hasPlanningKeyword(s.TextSnip) {
		return true
	}
	return s.TextLen > minTextLenForFlag
}

func hasPlanningKeyword(text string) bool {
	for _, kw := range planningKeywords {
		if strings.Contains(text, kw) {
			return true
		}
	}
	return false
}

// parseLastTurn scans the transcript JSONL and returns the aggregated
// summary of the last assistant turn (the run of assistant messages
// after the most recent user message). Returns zero-summary on empty
// transcripts or read errors.
func parseLastTurn(path string) (turnSummary, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return turnSummary{}, err
	}
	lines := strings.Split(strings.TrimRight(string(data), "\n"), "\n")

	// Walk from end backward to find the last user message; everything
	// after it is the current assistant turn.
	turnStart := 0
	for i := len(lines) - 1; i >= 0; i-- {
		var ev transcriptEvent
		if err := json.Unmarshal([]byte(lines[i]), &ev); err != nil {
			continue
		}
		if ev.Type == "user" {
			turnStart = i + 1
			break
		}
	}

	var summary turnSummary
	var textBuilder strings.Builder
	for i := turnStart; i < len(lines); i++ {
		var ev transcriptEvent
		if err := json.Unmarshal([]byte(lines[i]), &ev); err != nil {
			continue
		}
		if ev.Type != "assistant" {
			continue
		}
		if summary.Timestamp == "" {
			summary.Timestamp = ev.Timestamp
		}
		for _, b := range ev.Message.Content {
			switch b.Type {
			case "text":
				summary.TextLen += len(b.Text)
				if textBuilder.Len() < 4096 {
					textBuilder.WriteString(b.Text)
				}
			case "tool_use":
				if strings.HasPrefix(b.Name, hubSendToolPrefix) {
					summary.HubSent = true
				}
			}
		}
	}
	summary.TextSnip = textBuilder.String()
	if summary.Timestamp == "" {
		// Some transcripts omit per-event timestamps; fall back to file-mtime
		// so dedup key is still stable per-run.
		if fi, err := os.Stat(path); err == nil {
			summary.Timestamp = fi.ModTime().UTC().Format(time.RFC3339Nano)
		}
	}
	return summary, nil
}

// dedupLedgerPath returns the file used for idempotency tracking.
// Honors BOT_HQ_HOME for tests, mirrors gemma's sentinelsDir convention.
//
// TODO: prune entries older than dedupWindow on read (or write). v1 is
// no-prune — at the call rate this fires (only on actual OUTBOUND-MISS
// hits, dedup-window 60s) ledger growth is bounded for slice-5 window.
// Schedule trim if instrumentation stays past slice-5 close-out.
func dedupLedgerPath() (string, error) {
	if h := os.Getenv("BOT_HQ_HOME"); h != "" {
		return filepath.Join(h, "diag", "outbound-miss-dedup.jsonl"), nil
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(home, ".bot-hq", "diag", "outbound-miss-dedup.jsonl"), nil
}

type dedupEntry struct {
	Key string `json:"key"`
	TS  string `json:"ts"`
}

// alreadyFlaggedRecently returns true if (transcriptPath, turnTimestamp)
// has an entry in the ledger emitted within dedupWindow of now.
func alreadyFlaggedRecently(transcriptPath, turnTS string, now time.Time) bool {
	key := dedupKey(transcriptPath, turnTS)
	path, err := dedupLedgerPath()
	if err != nil {
		return false
	}
	data, err := os.ReadFile(path)
	if err != nil {
		return false
	}
	for _, line := range strings.Split(strings.TrimRight(string(data), "\n"), "\n") {
		if line == "" {
			continue
		}
		var e dedupEntry
		if err := json.Unmarshal([]byte(line), &e); err != nil {
			continue
		}
		if e.Key != key {
			continue
		}
		ts, err := time.Parse(time.RFC3339Nano, e.TS)
		if err != nil {
			continue
		}
		if now.Sub(ts) < dedupWindow {
			return true
		}
	}
	return false
}

// recordDedup appends a dedup entry to the ledger. Best-effort.
func recordDedup(transcriptPath, turnTS string, now time.Time) {
	path, err := dedupLedgerPath()
	if err != nil {
		return
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return
	}
	entry := dedupEntry{Key: dedupKey(transcriptPath, turnTS), TS: now.UTC().Format(time.RFC3339Nano)}
	data, err := json.Marshal(entry)
	if err != nil {
		return
	}
	f, err := os.OpenFile(path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o600)
	if err != nil {
		return
	}
	defer f.Close()
	f.Write(data)
	f.Write([]byte("\n"))
}

func dedupKey(transcriptPath, turnTS string) string {
	return transcriptPath + "|" + turnTS
}

// emitAlert opens the hub DB and inserts an OUTBOUND-MISS alert on
// behalf of the agent. The alert is broadcast (empty ToAgent) so the
// user sees the discipline gap directly without a routing layer.
func emitAlert(agentID, turnTS, textSnip string) error {
	dbPath := os.Getenv(dbPathEnvVar)
	if dbPath == "" {
		home, err := os.UserHomeDir()
		if err != nil {
			return err
		}
		dbPath = filepath.Join(home, ".bot-hq", "hub.db")
	}
	db, err := hub.OpenDB(dbPath)
	if err != nil {
		return err
	}
	defer db.Close()

	snip := textSnip
	if len(snip) > 160 {
		snip = snip[:160] + "…"
	}
	content := fmt.Sprintf("[OUTBOUND-MISS] agent %s emitted pane text at %s without a hub_send tool call. Excerpt: %q", agentID, turnTS, snip)
	_, err = db.InsertMessage(protocol.Message{
		FromAgent: agentID,
		Type:      protocol.MsgUpdate,
		Content:   content,
	})
	return err
}
