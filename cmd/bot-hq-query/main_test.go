package main

import (
	"strings"
	"testing"
)

// TestOneline locks the content-truncation helper. Phase J T1.6.
func TestOneline(t *testing.T) {
	cases := []struct {
		in   string
		want string
	}{
		{"short", "short"},
		{"line1\nline2", "line1 ⏎ line2"},
		{strings.Repeat("a", 100), strings.Repeat("a", 100)},
		{strings.Repeat("a", 200), strings.Repeat("a", 117) + "..."},
	}
	for _, c := range cases {
		got := oneline(c.in)
		if got != c.want {
			t.Errorf("oneline(%q):\n  got:  %q\n  want: %q", c.in, got, c.want)
		}
	}
}

// TestResolveDBPathHonorsBotHqHome locks BOT_HQ_HOME override.
func TestResolveDBPathHonorsBotHqHome(t *testing.T) {
	t.Setenv("BOT_HQ_HOME", "/tmp/test-bot-hq")
	got, err := resolveDBPath()
	if err != nil {
		t.Fatalf("resolveDBPath: %v", err)
	}
	want := "/tmp/test-bot-hq/hub.db"
	if got != want {
		t.Errorf("resolveDBPath:\n  got:  %q\n  want: %q", got, want)
	}
}

// TestResolveDBPathDefault locks default ~/.bot-hq/hub.db when env unset.
func TestResolveDBPathDefault(t *testing.T) {
	t.Setenv("BOT_HQ_HOME", "")
	got, err := resolveDBPath()
	if err != nil {
		t.Fatalf("resolveDBPath: %v", err)
	}
	if !strings.HasSuffix(got, "/.bot-hq/hub.db") {
		t.Errorf("resolveDBPath default = %q; expected suffix /.bot-hq/hub.db", got)
	}
}
