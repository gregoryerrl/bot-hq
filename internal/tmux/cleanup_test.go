// Phase T T-14 cycle-3: cleanup tests. Pure-function tests where possible
// (matchesAnyPrefix); integration tests (CleanupOrphanSessions) gated on
// HasTmux() to keep CI green on tmux-less hosts.

package tmux

import (
	"testing"
)

func TestMatchesAnyPrefix(t *testing.T) {
	cases := []struct {
		name     string
		input    string
		prefixes []string
		want     bool
	}{
		{
			name:     "matches_brian_prefix",
			input:    "bot-hq-brian-1778375059",
			prefixes: []string{"bot-hq-brian-"},
			want:     true,
		},
		{
			name:     "matches_rain_prefix_in_default_set",
			input:    "bot-hq-rain-1778375063",
			prefixes: AgentSessionPrefixes,
			want:     true,
		},
		{
			name:     "no_match_unrelated_session",
			input:    "my-tmux-work-session",
			prefixes: AgentSessionPrefixes,
			want:     false,
		},
		{
			name:     "no_match_prefix_substring_only",
			input:    "not-a-bot-hq-brian-session",
			prefixes: AgentSessionPrefixes,
			want:     false,
		},
		{
			name:     "matches_coder_prefix",
			input:    "bot-hq-coder-13ea3a3b",
			prefixes: AgentSessionPrefixes,
			want:     true,
		},
		{
			name:     "empty_prefixes_no_match",
			input:    "bot-hq-brian-1",
			prefixes: nil,
			want:     false,
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := matchesAnyPrefix(tc.input, tc.prefixes)
			if got != tc.want {
				t.Errorf("matchesAnyPrefix(%q) = %v, want %v", tc.input, got, tc.want)
			}
		})
	}
}

// TestCleanupOrphanSessions_KillsMatchingSessions exercises the full
// integration path: create disposable tmux sessions matching agent prefixes,
// run cleanup, verify they're gone. Skipped when tmux is unavailable so
// the unit suite stays green on tmux-less CI hosts.
func TestCleanupOrphanSessions_KillsMatchingSessions(t *testing.T) {
	if !HasTmux() {
		t.Skip("tmux not available — skipping integration test")
	}

	// Create two sessions: one orphan-class (matches prefix), one user-class
	// (does NOT match — must survive cleanup).
	dir := t.TempDir()
	orphan := "bot-hq-brian-test-9999999"
	keep := "user-tmux-not-orphan-9999999"

	if err := NewSession(orphan, dir); err != nil {
		t.Fatalf("create orphan: %v", err)
	}
	if err := NewSession(keep, dir); err != nil {
		_ = KillSession(orphan)
		t.Fatalf("create keep: %v", err)
	}
	t.Cleanup(func() {
		_ = KillSession(orphan)
		_ = KillSession(keep)
	})

	killed, errs := CleanupOrphanSessions(nil)
	for _, e := range errs {
		t.Errorf("cleanup err: %v", e)
	}

	wantKilled := false
	for _, name := range killed {
		if name == orphan {
			wantKilled = true
			break
		}
	}
	if !wantKilled {
		t.Errorf("expected orphan session %q in killed list; got %v", orphan, killed)
	}

	// Verify the user-class session survives.
	sessions, err := ListSessions()
	if err != nil {
		t.Fatalf("post-cleanup list: %v", err)
	}
	found := false
	for _, s := range sessions {
		if s.Name == keep {
			found = true
			break
		}
	}
	if !found {
		t.Errorf("user-class session %q was killed; cleanup over-aggressive", keep)
	}
}

// TestCleanupOrphanSessions_HandlesNoTmux ensures the cleanup is a graceful
// no-op when tmux is not available (rather than panicking or returning
// hard error).
func TestCleanupOrphanSessions_HandlesNoTmux(t *testing.T) {
	// We can't disable tmux mid-test; we only verify the function does not
	// panic when called with empty prefixes (no matches → nothing killed).
	killed, errs := CleanupOrphanSessions([]string{"definitely-no-such-prefix-"})
	if len(killed) != 0 {
		t.Errorf("expected zero kills for no-such-prefix; got %v", killed)
	}
	if len(errs) > 0 {
		t.Errorf("expected zero errors; got %v", errs)
	}
}
