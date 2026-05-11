// Phase T T-14 cycle-3 + Z-8h: tmux orphan-session cleanup.
//
// Pre-Z-3 design: every agent tmux pane was named with a Unix timestamp
// (bot-hq-brian-<ts>); on daemon restart, all such panes were
// orphaned-by-definition from the new daemon's routing, and
// CleanupOrphanSessions killed them blindly at startup. Rain msg 17419
// push-back A motivated this.
//
// Post-Z-8h: agent panes spawned inside a Z-3 session container are
// named with the session-id (bot-hq-brian-<session-id>); the Unix-ts
// naming remains only for legacy global-autostart-mode spawns. Orphan
// cleanup now spares any pane whose name contains an ACTIVE session-id
// (active = manifest with status=active). Truly-orphaned panes (Unix-ts
// names or session-ids of finalized sessions) are still killed.
//
// Failure is non-blocking (tmux may not be installed in CI / fresh
// install pre-ttyd): a kill error is logged and the next prefix is
// tried.

package tmux

import (
	"errors"
	"fmt"
	"os/exec"
	"strings"
)

// AgentSessionPrefixes lists the canonical bot-hq agent tmux session
// name-prefixes. Sessions matching any of these are eligible for
// orphan-cleanup; the per-session active-id list narrows the kill set
// further (Z-8h).
var AgentSessionPrefixes = []string{
	"bot-hq-brian-",
	"bot-hq-rain-",
	"bot-hq-emma-",
	"bot-hq-clive-",
	"bot-hq-coder-",
}

// CleanupOrphanSessions kills tmux sessions whose name starts with a
// known agent prefix UNLESS the suffix matches an active session-id
// (Z-8h). Pass nil for prefixes to use AgentSessionPrefixes (default).
// activeSessionIDs is the set of session-ids currently active (from
// sessions/*/manifest.md status=active). Panes named
// "<prefix><active-session-id>" are spared.
//
// Returns the list of killed session names plus errors (errors do NOT
// stop the loop — best-effort).
func CleanupOrphanSessions(prefixes []string, activeSessionIDs []string) ([]string, []error) {
	if prefixes == nil {
		prefixes = AgentSessionPrefixes
	}
	if !HasTmux() {
		return nil, nil
	}

	sessions, err := ListSessions()
	if err != nil {
		// "no server running" stderr means tmux is installed but no daemon
		// is up — equivalent to zero sessions from cleanup's perspective.
		var ee *exec.ExitError
		if errors.As(err, &ee) && strings.Contains(string(ee.Stderr), "no server running") {
			return nil, nil
		}
		return nil, []error{fmt.Errorf("list sessions: %w", err)}
	}

	activeSet := make(map[string]struct{}, len(activeSessionIDs))
	for _, id := range activeSessionIDs {
		if id != "" {
			activeSet[id] = struct{}{}
		}
	}

	var killed []string
	var errs []error
	for _, sess := range sessions {
		prefix, matched := matchedPrefix(sess.Name, prefixes)
		if !matched {
			continue
		}
		suffix := strings.TrimPrefix(sess.Name, prefix)
		if _, isActive := activeSet[suffix]; isActive {
			continue
		}
		if err := KillSession(sess.Name); err != nil {
			errs = append(errs, fmt.Errorf("kill %s: %w", sess.Name, err))
			continue
		}
		killed = append(killed, sess.Name)
	}
	return killed, errs
}

func matchedPrefix(name string, prefixes []string) (string, bool) {
	for _, p := range prefixes {
		if strings.HasPrefix(name, p) {
			return p, true
		}
	}
	return "", false
}

// matchesAnyPrefix is retained for backwards-compat with existing
// callers/tests that only need the boolean result.
func matchesAnyPrefix(name string, prefixes []string) bool {
	_, ok := matchedPrefix(name, prefixes)
	return ok
}
