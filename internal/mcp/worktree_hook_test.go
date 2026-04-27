package mcp

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

// makeWorktreeTestRepo builds a fresh repo + bare origin remote + a worktree
// branched from origin/main. Returns (mainRepo, worktreePath, bareRemote).
// The worktree has the freshness hook already installed.
func makeWorktreeTestRepo(t *testing.T) (string, string, string) {
	t.Helper()
	mainRepo := t.TempDir()
	bareRemote := t.TempDir()
	wtRoot := t.TempDir()
	worktreePath := filepath.Join(wtRoot, "wt-coder")

	mustGit(t, bareRemote, "init", "--bare", "-b", "main")

	mustGit(t, mainRepo, "init", "-b", "main")
	mustGit(t, mainRepo, "config", "user.email", "t@t.local")
	mustGit(t, mainRepo, "config", "user.name", "t")
	mustGit(t, mainRepo, "remote", "add", "origin", bareRemote)
	mustWriteFile(t, filepath.Join(mainRepo, "README"), "init")
	mustGit(t, mainRepo, "add", "README")
	mustGit(t, mainRepo, "commit", "-m", "init")
	mustGit(t, mainRepo, "push", "-u", "origin", "main")

	mustGit(t, mainRepo, "worktree", "add", "-b", "coder-test", worktreePath, "HEAD")
	mustGit(t, worktreePath, "config", "user.email", "t@t.local")
	mustGit(t, worktreePath, "config", "user.name", "t")

	if err := installWorktreeHooks(context.Background(), worktreePath); err != nil {
		t.Fatalf("install hook: %v", err)
	}
	return mainRepo, worktreePath, bareRemote
}

// seedClosedArcRepo provisions a worktree with a single seed arc.md whose
// frontmatter carries `Status: <status>`. Returns (worktreePath, arcRelPath).
// The seed commit itself is allowed (newly added file has no HEAD blob, so
// the closed-arc gate skips it).
func seedArcRepo(t *testing.T, status string) (string, string) {
	t.Helper()
	_, wt, _ := makeWorktreeTestRepo(t)
	arcRel := "docs/arcs/test-arc.md"
	arcAbs := filepath.Join(wt, arcRel)
	if err := os.MkdirAll(filepath.Dir(arcAbs), 0o755); err != nil {
		t.Fatalf("mkdir arc dir: %v", err)
	}
	body := fmt.Sprintf("# Arc: Test\n\nStatus: %s  | Branch: —\n\n## Body\n\nalpha\nbravo\ncharlie\n", status)
	mustWriteFile(t, arcAbs, body)
	mustGit(t, wt, "add", arcRel)
	mustGit(t, wt, "commit", "-m", "seed test arc")
	return wt, arcRel
}

func TestWorktreeHookInstallsOnSpawn(t *testing.T) {
	_, worktreePath, _ := makeWorktreeTestRepo(t)

	hookPath := filepath.Join(worktreePath, ".bot-hq-hooks", "pre-commit")
	info, err := os.Stat(hookPath)
	if err != nil {
		t.Fatalf("hook not found at %s: %v", hookPath, err)
	}
	if mode := info.Mode().Perm(); mode != 0o755 {
		t.Errorf("hook mode = %o, want 0755", mode)
	}
	body, err := os.ReadFile(hookPath)
	if err != nil {
		t.Fatalf("read hook: %v", err)
	}
	for _, want := range []string{
		"#!/usr/bin/env bash",
		"git fetch origin",
		"git merge-base --is-ancestor origin/main HEAD",
		"stale",
		"git rebase origin/main",
	} {
		if !strings.Contains(string(body), want) {
			t.Errorf("hook body missing %q\nbody:\n%s", want, body)
		}
	}

	// core.hooksPath is set in the worktree config (proves per-worktree
	// scope vs stomping the shared $GIT_COMMON_DIR/hooks).
	out := mustGitOut(t, worktreePath, "config", "--worktree", "core.hooksPath")
	if !strings.Contains(out, ".bot-hq-hooks") {
		t.Errorf("core.hooksPath = %q, want ...bot-hq-hooks", out)
	}
}

func TestPreCommitHookFailsOnStaleBase(t *testing.T) {
	mainRepo, worktreePath, bareRemote := makeWorktreeTestRepo(t)

	// Advance origin/main beyond the worktree's base via a side clone.
	// (Push from the main checkout would also work, but pushing through a
	// separate clone better mirrors the production failure mode: another
	// process advances origin/main while this worktree sits on the old SHA.)
	otherClone := t.TempDir()
	mustGit(t, otherClone, "clone", bareRemote, ".")
	mustGit(t, otherClone, "config", "user.email", "t@t.local")
	mustGit(t, otherClone, "config", "user.name", "t")
	mustWriteFile(t, filepath.Join(otherClone, "advance.txt"), "advance")
	mustGit(t, otherClone, "add", "advance.txt")
	mustGit(t, otherClone, "commit", "-m", "advance origin/main")
	mustGit(t, otherClone, "push", "origin", "main")

	// Stage a commit in the stale worktree and try to commit.
	mustWriteFile(t, filepath.Join(worktreePath, "coder-work.txt"), "coder")
	mustGit(t, worktreePath, "add", "coder-work.txt")
	cmd := exec.Command("git", "-C", worktreePath, "commit", "-m", "coder commit on stale base")
	out, err := cmd.CombinedOutput()
	if err == nil {
		t.Fatalf("expected commit to fail on stale base, succeeded:\n%s", out)
	}
	if !strings.Contains(string(out), "stale") {
		t.Errorf("expected error to mention 'stale', got:\n%s", out)
	}

	// Sanity: prove the gate is the cause — rebase + retry should now
	// succeed (origin/main becomes ancestor of HEAD post-rebase). Stash
	// the staged-but-rejected commit first so rebase can apply.
	mustGit(t, worktreePath, "stash", "--include-untracked")
	mustGit(t, worktreePath, "fetch", "origin")
	mustGit(t, worktreePath, "rebase", "origin/main")
	mustGit(t, worktreePath, "stash", "pop")
	mustGit(t, worktreePath, "add", "coder-work.txt")
	if out, err := exec.Command("git", "-C", worktreePath, "commit", "-m", "coder commit post-rebase").CombinedOutput(); err != nil {
		t.Errorf("commit after rebase failed:\n%s\n%v", out, err)
	}

	_ = mainRepo
}

func TestPreCommitHookPassesOnFreshBase(t *testing.T) {
	_, worktreePath, _ := makeWorktreeTestRepo(t)

	mustWriteFile(t, filepath.Join(worktreePath, "fresh.txt"), "fresh")
	mustGit(t, worktreePath, "add", "fresh.txt")
	if out, err := exec.Command("git", "-C", worktreePath, "commit", "-m", "fresh-base commit").CombinedOutput(); err != nil {
		t.Fatalf("expected commit on fresh base to succeed:\n%s\n%v", out, err)
	}
}

// H-6 gate tests --------------------------------------------------------

func TestClosedArcRetroactiveEditRejected(t *testing.T) {
	wt, arcRel := seedArcRepo(t, "closed")
	arcAbs := filepath.Join(wt, arcRel)
	body, err := os.ReadFile(arcAbs)
	if err != nil {
		t.Fatalf("read seed: %v", err)
	}
	edited := strings.Replace(string(body), "bravo", "MODIFIED", 1)
	if edited == string(body) {
		t.Fatalf("test setup broken: bravo not present in seed")
	}
	mustWriteFile(t, arcAbs, edited)
	mustGit(t, wt, "add", arcRel)
	out, err := exec.Command("git", "-C", wt, "commit", "-m", "retroactive edit").CombinedOutput()
	if err == nil {
		t.Fatalf("expected commit to fail; succeeded:\n%s", out)
	}
	got := string(out)
	if !strings.Contains(got, "closed arc") || !strings.Contains(got, "append-only") {
		t.Errorf("expected closed-arc/append-only error, got:\n%s", got)
	}
	if !strings.Contains(got, arcRel) {
		t.Errorf("expected error to name the offending file %q, got:\n%s", arcRel, got)
	}
}

func TestClosedArcAppendPasses(t *testing.T) {
	wt, arcRel := seedArcRepo(t, "closed")
	arcAbs := filepath.Join(wt, arcRel)
	body, err := os.ReadFile(arcAbs)
	if err != nil {
		t.Fatalf("read seed: %v", err)
	}
	appended := string(body) + "delta\necho\n"
	mustWriteFile(t, arcAbs, appended)
	mustGit(t, wt, "add", arcRel)
	if out, err := exec.Command("git", "-C", wt, "commit", "-m", "append to closed arc").CombinedOutput(); err != nil {
		t.Fatalf("expected EOF append on closed arc to succeed:\n%s\n%v", out, err)
	}
}

func TestOpenArcEditPasses(t *testing.T) {
	wt, arcRel := seedArcRepo(t, "open")
	arcAbs := filepath.Join(wt, arcRel)
	body, err := os.ReadFile(arcAbs)
	if err != nil {
		t.Fatalf("read seed: %v", err)
	}
	edited := strings.Replace(string(body), "bravo", "MODIFIED", 1)
	mustWriteFile(t, arcAbs, edited)
	mustGit(t, wt, "add", arcRel)
	if out, err := exec.Command("git", "-C", wt, "commit", "-m", "open arc edit").CombinedOutput(); err != nil {
		t.Fatalf("expected open-arc edit to succeed (gate scoped to Status: closed):\n%s\n%v", out, err)
	}
}

func TestNonArcMdEditPasses(t *testing.T) {
	_, wt, _ := makeWorktreeTestRepo(t)
	nonArcRel := "docs/notes.md"
	nonArcAbs := filepath.Join(wt, nonArcRel)
	if err := os.MkdirAll(filepath.Dir(nonArcAbs), 0o755); err != nil {
		t.Fatalf("mkdir: %v", err)
	}
	// Frontmatter literally says `Status: closed` — but the file is NOT
	// under docs/arcs/, so the gate must skip it (path scope, not content
	// scope).
	mustWriteFile(t, nonArcAbs, "Status: closed\n\nalpha\nbravo\ncharlie\n")
	mustGit(t, wt, "add", nonArcRel)
	mustGit(t, wt, "commit", "-m", "seed non-arc md")
	mustWriteFile(t, nonArcAbs, "Status: closed\n\nalpha\nMODIFIED\ncharlie\n")
	mustGit(t, wt, "add", nonArcRel)
	if out, err := exec.Command("git", "-C", wt, "commit", "-m", "non-arc md mid-edit").CombinedOutput(); err != nil {
		t.Fatalf("expected non-arc md edit to succeed (gate scoped to docs/arcs/):\n%s\n%v", out, err)
	}
}

func mustGit(t *testing.T, dir string, args ...string) {
	t.Helper()
	full := append([]string{"-C", dir}, args...)
	cmd := exec.Command("git", full...)
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("git %v failed: %v\n%s", args, err, out)
	}
}

func mustGitOut(t *testing.T, dir string, args ...string) string {
	t.Helper()
	full := append([]string{"-C", dir}, args...)
	out, err := exec.Command("git", full...).Output()
	if err != nil {
		t.Fatalf("git %v failed: %v", args, err)
	}
	return strings.TrimSpace(string(out))
}

func mustWriteFile(t *testing.T, path, content string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatalf("write %s: %v", path, err)
	}
}
