package projectctx

import (
	"os"
	"os/exec"
	"path/filepath"
	"testing"
)

func TestDetectFrom_envWins(t *testing.T) {
	got := DetectFrom("custom-project", "/tmp", "/tmp/canon")
	if got != "custom-project" {
		t.Errorf("env should win: got %q", got)
	}
}

func TestDetectFrom_envWhitespaceTrimmed(t *testing.T) {
	got := DetectFrom("  trimmed  ", "/tmp", "/tmp/canon")
	if got != "trimmed" {
		t.Errorf("env whitespace should be trimmed: got %q", got)
	}
}

func TestDetectFrom_emptyEnvFallsThrough(t *testing.T) {
	// No env, no matching cwd → default.
	got := DetectFrom("", "/nonexistent", "/nonexistent")
	if got != DefaultProject {
		t.Errorf("empty signals should default to %q: got %q", DefaultProject, got)
	}
}

func TestDetectFrom_cwdInferenceWithRegisteredProject(t *testing.T) {
	canon := t.TempDir()
	if err := os.MkdirAll(filepath.Join(canon, "projects"), 0o755); err != nil {
		t.Fatal(err)
	}
	// Register a fake project named "bot-hq" via projects/<n>.yaml.
	if err := os.WriteFile(filepath.Join(canon, "projects", "bot-hq.yaml"), []byte("project_name: bot-hq\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	// Build a fake repo dir with .git and origin pointing at bot-hq.
	repo := t.TempDir()
	cmd := exec.Command("git", "init", "-q", repo)
	if err := cmd.Run(); err != nil {
		t.Skipf("git not available: %v", err)
	}
	cmd = exec.Command("git", "-C", repo, "remote", "add", "origin", "https://github.com/gregoryerrl/bot-hq.git")
	if err := cmd.Run(); err != nil {
		t.Fatal(err)
	}
	got := DetectFrom("", repo, canon)
	if got != "bot-hq" {
		t.Errorf("cwd-inference should resolve to 'bot-hq': got %q", got)
	}
}

func TestDetectFrom_cwdInferenceUnregisteredFallsBack(t *testing.T) {
	canon := t.TempDir()
	repo := t.TempDir()
	if err := exec.Command("git", "init", "-q", repo).Run(); err != nil {
		t.Skipf("git not available: %v", err)
	}
	if err := exec.Command("git", "-C", repo, "remote", "add", "origin", "https://github.com/foo/unregistered.git").Run(); err != nil {
		t.Fatal(err)
	}
	got := DetectFrom("", repo, canon)
	if got != DefaultProject {
		t.Errorf("unregistered project should default: got %q", got)
	}
}
