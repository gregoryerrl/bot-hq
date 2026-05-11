package protocol

import (
	"path/filepath"
	"strings"
	"testing"
)

func TestAgentStatePath_HappyPath(t *testing.T) {
	got := AgentStatePath("/canon", "z-3-foo-abc123", "brian")
	want := filepath.Join("/canon", "sessions", "z-3-foo-abc123", "brian", "state.json")
	if got != want {
		t.Errorf("AgentStatePath=%q want %q", got, want)
	}
}

func TestAgentStatePath_EmptyArgsReturnEmpty(t *testing.T) {
	cases := []struct {
		name      string
		root, sid, agent string
	}{
		{"empty root", "", "sid", "brian"},
		{"empty sid", "/canon", "", "brian"},
		{"empty agent", "/canon", "sid", ""},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := AgentStatePath(tc.root, tc.sid, tc.agent); got != "" {
				t.Errorf("AgentStatePath(%q, %q, %q)=%q want empty", tc.root, tc.sid, tc.agent, got)
			}
		})
	}
}

func TestAgentStatePath_SessionsBoundShape(t *testing.T) {
	// Z-3 substrate: sessions-as-containers anchor — path must contain
	// /sessions/<sid>/<agent>/ shape (anti-regression-lock against
	// regression to top-level <agent>/last_state.json shape).
	got := AgentStatePath("/canon", "scope-abc", "rain")
	if !strings.Contains(got, "/sessions/scope-abc/rain/state.json") {
		t.Errorf("Z-3 session-bound shape missing in %q", got)
	}
	if strings.Contains(got, "last_state.json") {
		t.Errorf("legacy last_state.json shape present in %q (Z-3 ratchet broken)", got)
	}
}

func TestCanonRoot_HonorsBotHQHome(t *testing.T) {
	t.Setenv("BOT_HQ_HOME", "/tmp/test-canon")
	got, err := CanonRoot()
	if err != nil {
		t.Fatalf("CanonRoot err: %v", err)
	}
	if got != "/tmp/test-canon" {
		t.Errorf("CanonRoot=%q want /tmp/test-canon (BOT_HQ_HOME honored)", got)
	}
}
