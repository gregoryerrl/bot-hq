// Phase T T-14 cycle-3: tmux orphan-session cleanup.
//
// On bot-hq daemon-restart, the new daemon spawns fresh agent tmux sessions
// with new Unix-timestamp suffixes (e.g., bot-hq-brian-1778375059 → bot-hq-
// brian-1778378XYZ). The OLD pre-restart sessions remain attached in tmux
// but are orphaned from the new daemon's hub-coord routing — Rain msg 17419
// push-back A documented this as user-confusion class (orphaned panes look
// alive but won't receive new messages).
//
// CleanupOrphanSessions kills any tmux session whose name starts with a
// known agent prefix BEFORE the daemon spawns fresh ones. Failure is
// non-blocking (tmux may not be installed in CI / fresh install pre-ttyd):
// a kill error is logged and the next prefix is tried.

package tmux

import (
	"errors"
	"fmt"
	"os/exec"
	"strings"
)

// AgentSessionPrefixes lists the canonical bot-hq agent tmux session
// name-prefixes. Sessions matching any of these are orphan-class on
// daemon-restart and safe to kill.
var AgentSessionPrefixes = []string{
	"bot-hq-brian-",
	"bot-hq-rain-",
	"bot-hq-emma-",
	"bot-hq-clive-",
	"bot-hq-coder-",
}

// CleanupOrphanSessions kills any existing tmux session whose name has one
// of the given prefixes. Returns the list of killed session names plus any
// errors encountered (errors do NOT stop the loop — best-effort cleanup).
//
// Pass nil for prefixes to use AgentSessionPrefixes (the default set).
func CleanupOrphanSessions(prefixes []string) ([]string, []error) {
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

	var killed []string
	var errs []error
	for _, sess := range sessions {
		if !matchesAnyPrefix(sess.Name, prefixes) {
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

func matchesAnyPrefix(name string, prefixes []string) bool {
	for _, p := range prefixes {
		if strings.HasPrefix(name, p) {
			return true
		}
	}
	return false
}
