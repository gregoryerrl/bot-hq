package gemma

import (
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
)

// makeRepoWithMergedBranch creates a tmp git repo with:
//   - main branch holding 1 initial commit
//   - feature branch X holding a commit that's then ff-merged to main
//   - origin remote configured (file:// pointing at a bare clone) so
//     `origin/main` and `origin/<branch>` refs exist
//
// Returns (repoDir, mainSHA, mergedBranchName, mergedBranchTipSHA).
func makeRepoWithMergedBranch(t *testing.T) (repoDir, mainSHA, branchName, branchSHA string) {
	t.Helper()
	repoDir = t.TempDir()
	bareDir := t.TempDir()

	// Init bare remote
	mustRun(t, bareDir, "git", "init", "--bare")

	// Init working repo + connect remote
	mustRun(t, repoDir, "git", "init", "-b", "main")
	mustRun(t, repoDir, "git", "config", "user.email", "test@test.local")
	mustRun(t, repoDir, "git", "config", "user.name", "test")
	mustRun(t, repoDir, "git", "remote", "add", "origin", bareDir)

	// Initial commit on main
	mustWrite(t, filepath.Join(repoDir, "README"), "init")
	mustRun(t, repoDir, "git", "add", "README")
	mustRun(t, repoDir, "git", "commit", "-m", "init")
	mainSHA = strings.TrimSpace(mustOut(t, repoDir, "git", "rev-parse", "HEAD"))

	// Feature branch with a commit
	branchName = "feature/already-merged"
	mustRun(t, repoDir, "git", "checkout", "-b", branchName)
	mustWrite(t, filepath.Join(repoDir, "feature.txt"), "feat")
	mustRun(t, repoDir, "git", "add", "feature.txt")
	mustRun(t, repoDir, "git", "commit", "-m", "feat")
	branchSHA = strings.TrimSpace(mustOut(t, repoDir, "git", "rev-parse", "HEAD"))

	// ff-merge feature back to main
	mustRun(t, repoDir, "git", "checkout", "main")
	mustRun(t, repoDir, "git", "merge", "--ff-only", branchName)

	// Push both branches to origin so origin/main + origin/<branch> exist
	mustRun(t, repoDir, "git", "push", "origin", "main")
	mustRun(t, repoDir, "git", "push", "origin", branchName)

	mainSHA = strings.TrimSpace(mustOut(t, repoDir, "git", "rev-parse", "origin/main"))
	return repoDir, mainSHA, branchName, branchSHA
}

func mustRun(t *testing.T, dir string, args ...string) {
	t.Helper()
	cmd := exec.Command(args[0], args[1:]...)
	cmd.Dir = dir
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("git %v failed: %v\n%s", args, err, out)
	}
}

func mustOut(t *testing.T, dir string, args ...string) string {
	t.Helper()
	cmd := exec.Command(args[0], args[1:]...)
	cmd.Dir = dir
	out, err := cmd.Output()
	if err != nil {
		t.Fatalf("git %v failed: %v", args, err)
	}
	return string(out)
}

func mustWrite(t *testing.T, path, content string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
}

// TestDocDriftSentinelDetectsMergedBranch locks H-23 — when an open
// arc.md references a branch (`feature/already-merged`) whose tip is an
// ancestor of origin/main, the sentinel emits a "merged-branch"
// observation.
func TestDocDriftSentinelDetectsMergedBranch(t *testing.T) {
	repoDir, _, branchName, _ := makeRepoWithMergedBranch(t)

	arcsDir := filepath.Join(repoDir, "docs", "arcs")
	if err := os.MkdirAll(arcsDir, 0o755); err != nil {
		t.Fatal(err)
	}
	arcContent := "# Test arc\n\nStatus: open  | Branch: `" + branchName + "`\n\nReferences `" + branchName + "` for the slice.\n"
	if err := os.WriteFile(filepath.Join(arcsDir, "test-arc.md"), []byte(arcContent), 0o644); err != nil {
		t.Fatal(err)
	}

	obs, err := ScanArcsForDocDrift(arcsDir, repoDir)
	if err != nil {
		t.Fatalf("scan: %v", err)
	}
	found := false
	for _, o := range obs {
		if o.Kind == "merged-branch" && o.Reference == branchName {
			found = true
		}
	}
	if !found {
		t.Errorf("expected merged-branch observation for %q, got %+v", branchName, obs)
	}
}

// TestDocDriftSentinelDetectsAncestorSHA locks H-23 — when an open
// arc.md references a backtick-fenced SHA that's an ancestor of
// origin/main, the sentinel emits an "ancestor-sha" observation.
func TestDocDriftSentinelDetectsAncestorSHA(t *testing.T) {
	repoDir, mainSHA, _, _ := makeRepoWithMergedBranch(t)
	shortSHA := mainSHA[:7]

	arcsDir := filepath.Join(repoDir, "docs", "arcs")
	if err := os.MkdirAll(arcsDir, 0o755); err != nil {
		t.Fatal(err)
	}
	arcContent := "# Test arc\n\nStatus: open  | Branch: main@`" + shortSHA + "`\n\nThe slice merged at `" + shortSHA + "`.\n"
	if err := os.WriteFile(filepath.Join(arcsDir, "test-arc.md"), []byte(arcContent), 0o644); err != nil {
		t.Fatal(err)
	}

	obs, err := ScanArcsForDocDrift(arcsDir, repoDir)
	if err != nil {
		t.Fatalf("scan: %v", err)
	}
	found := false
	for _, o := range obs {
		if o.Kind == "ancestor-sha" && o.Reference == shortSHA {
			found = true
		}
	}
	if !found {
		t.Errorf("expected ancestor-sha observation for %q, got %+v", shortSHA, obs)
	}
}

// TestDocDriftSentinelIgnoresClosedArcs locks H-23 — closed arcs
// (Status: closed) are out-of-scope per the convention. Even if they
// reference a merged branch or ancestor SHA, the scan must skip them.
func TestDocDriftSentinelIgnoresClosedArcs(t *testing.T) {
	repoDir, mainSHA, branchName, _ := makeRepoWithMergedBranch(t)
	shortSHA := mainSHA[:7]

	arcsDir := filepath.Join(repoDir, "docs", "arcs")
	if err := os.MkdirAll(arcsDir, 0o755); err != nil {
		t.Fatal(err)
	}
	arcContent := "# Closed arc\n\nStatus: closed | Branch: `" + branchName + "` | Merged at `" + shortSHA + "`\n"
	if err := os.WriteFile(filepath.Join(arcsDir, "closed-arc.md"), []byte(arcContent), 0o644); err != nil {
		t.Fatal(err)
	}

	obs, err := ScanArcsForDocDrift(arcsDir, repoDir)
	if err != nil {
		t.Fatalf("scan: %v", err)
	}
	if len(obs) != 0 {
		t.Errorf("closed arc must produce zero observations; got %+v", obs)
	}
}

// TestDocDriftSentinelEmitWritesLedgerWhenDryRunActive locks the dry-run
// dispatch path — EmitDocDriftObservations writes to the docdrift ledger
// when docDriftDryRunActive is true.
func TestDocDriftSentinelEmitWritesLedgerWhenDryRunActive(t *testing.T) {
	home := t.TempDir()
	t.Setenv("BOT_HQ_HOME", home)

	prev := docDriftDryRunActive
	docDriftDryRunActive = true
	defer func() { docDriftDryRunActive = prev }()

	EmitDocDriftObservations([]DocDriftObservation{
		{ArcPath: "/abs/docs/arcs/test.md", Kind: "merged-branch", Reference: "brian/foo"},
		{ArcPath: "/abs/docs/arcs/test.md", Kind: "ancestor-sha", Reference: "abc1234"},
	})

	path := filepath.Join(home, "sentinels", "docdrift-dryrun.log")
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("ledger not created: %v", err)
	}
	content := string(data)
	if !strings.Contains(content, "merged-branch") || !strings.Contains(content, "brian/foo") {
		t.Errorf("ledger missing merged-branch entry; got: %q", content)
	}
	if !strings.Contains(content, "ancestor-sha") || !strings.Contains(content, "abc1234") {
		t.Errorf("ledger missing ancestor-sha entry; got: %q", content)
	}
}
