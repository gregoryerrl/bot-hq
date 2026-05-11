package discord

import (
	"os"
	"strings"

	"github.com/gregoryerrl/bot-hq/internal/sessions"
)

// RegisterSessionThread records the bidirectional session_id↔thread_id
// mapping for outbound routing (Z-7b: route session-scoped hub
// messages to the session's thread) and inbound tagging (Z-7c: tag
// inbound thread messages with their session_id).
//
// Called by the daemon-side session-open hook after a thread is
// successfully created.
func (b *Bot) RegisterSessionThread(sessionID, threadID string) {
	if sessionID == "" || threadID == "" {
		return
	}
	b.threadsMu.Lock()
	defer b.threadsMu.Unlock()
	if b.threadByID == nil {
		b.threadByID = make(map[string]string)
	}
	if b.sessionByThread == nil {
		b.sessionByThread = make(map[string]string)
	}
	b.threadByID[sessionID] = threadID
	b.sessionByThread[threadID] = sessionID
}

// UnregisterSessionThread removes a session↔thread mapping. Called by
// the finalize hook after archive; safe to call with unknown ids.
func (b *Bot) UnregisterSessionThread(sessionID string) {
	if sessionID == "" {
		return
	}
	b.threadsMu.Lock()
	defer b.threadsMu.Unlock()
	if tid, ok := b.threadByID[sessionID]; ok {
		delete(b.sessionByThread, tid)
	}
	delete(b.threadByID, sessionID)
}

// threadForSession returns the registered thread-id for a session, or
// empty if unknown. Caller falls back to hub channel.
func (b *Bot) threadForSession(sessionID string) string {
	if sessionID == "" {
		return ""
	}
	b.threadsMu.RLock()
	defer b.threadsMu.RUnlock()
	return b.threadByID[sessionID]
}

// sessionForThread returns the registered session-id for a thread, or
// empty if the channel-id isn't a known session-thread.
func (b *Bot) sessionForThread(threadID string) string {
	if threadID == "" {
		return ""
	}
	b.threadsMu.RLock()
	defer b.threadsMu.RUnlock()
	return b.sessionByThread[threadID]
}

// bootstrapThreadRegistry walks the sessions directory at Start and
// loads every active session's discord_thread_id into the registry, so
// outbound routing + inbound tagging work across daemon restarts.
//
// Best-effort: filesystem errors are tolerated silently. Sessions
// without a thread-id (pre-Z-7 OR Discord-disabled-when-opened) are
// skipped — their hub-rows just route to the hub channel.
func (b *Bot) bootstrapThreadRegistry() {
	root := sessions.SessionsDir()
	entries, err := os.ReadDir(root)
	if err != nil {
		return
	}
	for _, e := range entries {
		if !e.IsDir() {
			continue
		}
		// Skip closed-session archive dirs (sessions are renamed to
		// <id>__closed_<timestamp> by some finalize paths).
		if strings.Contains(e.Name(), "__closed_") {
			continue
		}
		m, rerr := sessions.ReadManifest(e.Name())
		if rerr != nil {
			continue
		}
		if m.DiscordThreadID != "" && m.Status == "active" {
			b.RegisterSessionThread(m.ID, m.DiscordThreadID)
		}
	}
}
