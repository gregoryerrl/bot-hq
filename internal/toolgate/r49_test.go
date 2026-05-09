package toolgate

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestIsScopeLockDoc_phaseT(t *testing.T) {
	home, _ := os.UserHomeDir()
	cases := []struct {
		path string
		want bool
	}{
		{filepath.Join(home, ".bot-hq", "phase", "phase-t.md"), true},
		{filepath.Join(home, ".bot-hq", "phase", "phase-s.md"), true},
		{filepath.Join(home, ".bot-hq", "phase", "subdir", "nested.md"), true},
		{filepath.Join(home, ".bot-hq", "ratchets", "active.md"), false},
		{filepath.Join(home, "Projects", "bot-hq", "main.go"), false},
		{filepath.Join(home, ".bot-hq", "phase", "phase-t.txt"), false}, // wrong extension
	}
	for _, tc := range cases {
		got := IsScopeLockDoc(tc.path)
		if got != tc.want {
			t.Errorf("IsScopeLockDoc(%q) = %v, want %v", tc.path, got, tc.want)
		}
	}
}

func TestR49PreSealAudit_passOnAllValidAnchors(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("HOME", dir)
	scopePath := filepath.Join(dir, ".bot-hq", "phase", "phase-test.md")
	if err := os.MkdirAll(filepath.Dir(scopePath), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}

	// Create the existing target file that the scope-lock-doc cites
	targetPath := filepath.Join(dir, "Users", "fake", "doc.md")
	if err := os.MkdirAll(filepath.Dir(targetPath), 0o755); err != nil {
		t.Fatalf("mkdir target: %v", err)
	}
	if err := os.WriteFile(targetPath, []byte("hello"), 0o644); err != nil {
		t.Fatalf("write target: %v", err)
	}

	// scope-lock-doc cites the existing file
	content := []byte("# Test scope-lock\nCites " + targetPath + " (which exists).")

	v := R49PreSealAudit(context.Background(), scopePath, content)
	if v.ShouldBlock {
		t.Errorf("expected !ShouldBlock; reason: %s", v.Reason)
	}
}

func TestR49PreSealAudit_invalidAnchor_warnMode_doesNotBlock(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("HOME", dir)
	scopePath := filepath.Join(dir, ".bot-hq", "phase", "phase-test.md")
	if err := os.MkdirAll(filepath.Dir(scopePath), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}

	t.Setenv("BOT_HQ_R49_MODE", R49ModeWarn)

	content := []byte("Cites /tmp/totally-nonexistent-xyz-abc.md")
	v := R49PreSealAudit(context.Background(), scopePath, content)

	if v.Result.Invalid < 1 {
		t.Errorf("expected >=1 invalid anchor; got %d", v.Result.Invalid)
	}
	if v.ShouldBlock {
		t.Errorf("warn mode should not block; got ShouldBlock=true")
	}
	if !strings.Contains(v.Reason, "R49 PRE-SEAL-AUDIT findings") {
		t.Errorf("expected R49 reason in warn output; got: %s", v.Reason)
	}
}

func TestR49PreSealAudit_invalidAnchor_blockMode_blocks(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("HOME", dir)
	scopePath := filepath.Join(dir, ".bot-hq", "phase", "phase-test.md")
	if err := os.MkdirAll(filepath.Dir(scopePath), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}

	t.Setenv("BOT_HQ_R49_MODE", R49ModeBlock)

	content := []byte("Cites /tmp/totally-nonexistent-xyz-abc.md")
	v := R49PreSealAudit(context.Background(), scopePath, content)

	if !v.ShouldBlock {
		t.Errorf("block mode should block on invalid anchor; got ShouldBlock=false")
	}
	if !strings.Contains(v.Reason, "R49 PRE-SEAL-AUDIT findings") {
		t.Errorf("expected R49 reason; got: %s", v.Reason)
	}
}

func TestR49PreSealAudit_nonScopeLockPath_skipsAudit(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("HOME", dir)
	nonScopePath := filepath.Join(dir, "Projects", "bot-hq", "main.go")

	t.Setenv("BOT_HQ_R49_MODE", R49ModeBlock)

	// Even with invalid anchors, non-scope-lock path is skipped
	content := []byte("Cites /tmp/totally-nonexistent-xyz.md")
	v := R49PreSealAudit(context.Background(), nonScopePath, content)

	if v.ShouldBlock {
		t.Errorf("non-scope-lock path should not be audited; got ShouldBlock=true")
	}
	if v.Reason != "" {
		t.Errorf("non-scope-lock path should produce empty reason; got: %s", v.Reason)
	}
}

func TestR49PreSealAudit_defaultModeIsWarn(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("HOME", dir)
	scopePath := filepath.Join(dir, ".bot-hq", "phase", "phase-test.md")
	if err := os.MkdirAll(filepath.Dir(scopePath), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}

	// Explicitly unset BOT_HQ_R49_MODE
	os.Unsetenv("BOT_HQ_R49_MODE")

	content := []byte("Cites /tmp/totally-nonexistent-xyz-abc.md")
	v := R49PreSealAudit(context.Background(), scopePath, content)

	if v.Mode != R49ModeWarn {
		t.Errorf("default mode = %q, want %q", v.Mode, R49ModeWarn)
	}
	if v.ShouldBlock {
		t.Errorf("default mode should be warn (no block)")
	}
}

func TestR49PreSealAudit_truncatesLongFindingList(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("HOME", dir)
	scopePath := filepath.Join(dir, ".bot-hq", "phase", "phase-test.md")
	if err := os.MkdirAll(filepath.Dir(scopePath), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}

	// Generate content with 15 invalid file-path anchors
	var sb strings.Builder
	for i := 0; i < 15; i++ {
		sb.WriteString("Cites /tmp/missing-file-")
		sb.WriteString(string(rune('A' + i)))
		sb.WriteString(".md ")
	}

	v := R49PreSealAudit(context.Background(), scopePath, []byte(sb.String()))
	if v.Result.Invalid < 10 {
		t.Errorf("expected >=10 invalid; got %d", v.Result.Invalid)
	}
	if !strings.Contains(v.Reason, "more invalid; truncated") {
		t.Errorf("expected truncation marker in reason; got: %s", v.Reason)
	}
}
