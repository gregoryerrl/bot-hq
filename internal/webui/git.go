package webui

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

// canonicalGitDirs is the set of canonical-store top-level dirs that get
// per-dir `.git/` audit infra. Lazy-initialized on first daemon write per
// scope-lock OQ-1 LOCKED.
//
// Each dir gets its own git repo so commits scope cleanly to that
// canonical sub-class (rules vs phase docs vs project plans). Keeps the
// audit log narrow + revert decisions per-class.
var canonicalGitDirs = []string{"phase", "ratchets", "rules", "projects"}

// ensureCanonicalGit lazy-initializes a per-dir `.git/` for the canonical
// dir containing the given relPath, if not already initialized. relPath
// is canonical-store-relative (e.g., "rules/general.yaml").
//
// The git repo lives at `<root>/<top-dir>/.git/` so commits + history are
// scoped to that dir tree only.
//
// Returns (gitDirAbs, isNewlyInitialized, err). Idempotent; second call
// returns isNewlyInitialized=false.
func ensureCanonicalGit(root, relPath string) (string, bool, error) {
	topDir := canonicalTopDir(relPath)
	if topDir == "" {
		// File at root level (e.g., README.md, discipline-log.md) — no
		// per-dir git, audit handled separately if needed.
		return "", false, nil
	}
	// Only init for known canonical dirs; avoid arbitrary-dir creep.
	known := false
	for _, d := range canonicalGitDirs {
		if d == topDir {
			known = true
			break
		}
	}
	if !known {
		return "", false, nil
	}
	gitDir := filepath.Join(root, topDir, ".git")
	if _, err := os.Stat(gitDir); err == nil {
		return gitDir, false, nil // already initialized
	} else if !errors.Is(err, os.ErrNotExist) {
		return "", false, err
	}
	repoDir := filepath.Join(root, topDir)
	if err := os.MkdirAll(repoDir, 0o755); err != nil {
		return "", false, fmt.Errorf("mkdir %s: %w", repoDir, err)
	}
	cmd := exec.Command("git", "init", "--quiet", repoDir)
	if out, err := cmd.CombinedOutput(); err != nil {
		return "", false, fmt.Errorf("git init %s: %w (output: %s)", repoDir, err, string(out))
	}
	// Configure so we don't depend on user's global git identity for
	// audit commits — fixed identity makes every audit commit clearly
	// traceable as "from the bot-hq daemon".
	for _, kv := range [][2]string{
		{"user.name", "bot-hq daemon"},
		{"user.email", "daemon@bot-hq.local"},
	} {
		cmd := exec.Command("git", "-C", repoDir, "config", kv[0], kv[1])
		if out, err := cmd.CombinedOutput(); err != nil {
			return "", false, fmt.Errorf("git config %s: %w (output: %s)", kv[0], err, string(out))
		}
	}
	return gitDir, true, nil
}

// canonicalTopDir returns the first path component of relPath, or empty
// string if relPath is at root level (no slashes).
func canonicalTopDir(relPath string) string {
	parts := strings.SplitN(relPath, "/", 2)
	if len(parts) < 2 {
		return ""
	}
	return parts[0]
}

// commitCanonicalChange stages + commits the given relPath into its
// per-dir git repo with the supplied author identity ("user@webui" /
// "clive@webui" / etc.) and message. Idempotent on no-change (returns
// "" SHA + nil err for the no-change case).
//
// Caller is responsible for calling ensureCanonicalGit before this; the
// commit fails fast if the per-dir .git doesn't exist.
func commitCanonicalChange(root, relPath, author, message string) (string, error) {
	topDir := canonicalTopDir(relPath)
	if topDir == "" {
		return "", nil // root-level file — no per-dir git
	}
	repoDir := filepath.Join(root, topDir)
	if _, err := os.Stat(filepath.Join(repoDir, ".git")); err != nil {
		return "", fmt.Errorf("git not initialized at %s", repoDir)
	}
	// Strip the topDir prefix from relPath so git sees a repo-relative path.
	repoRel := strings.TrimPrefix(relPath, topDir+"/")
	// Stage.
	cmd := exec.Command("git", "-C", repoDir, "add", "--", repoRel)
	if out, err := cmd.CombinedOutput(); err != nil {
		return "", fmt.Errorf("git add: %w (output: %s)", err, string(out))
	}
	// Commit. Use --allow-empty=false (default) but tolerate "nothing to
	// commit" as no-change return.
	cmd = exec.Command("git", "-C", repoDir,
		"-c", fmt.Sprintf("user.name=%s", author),
		"-c", fmt.Sprintf("user.email=%s@webui", author),
		"commit", "--quiet", "-m", message)
	out, err := cmd.CombinedOutput()
	if err != nil {
		// If the only "failure" is nothing-to-commit, treat as no-op.
		if strings.Contains(string(out), "nothing to commit") {
			return "", nil
		}
		return "", fmt.Errorf("git commit: %w (output: %s)", err, string(out))
	}
	// Capture the new HEAD SHA.
	sha, err := exec.Command("git", "-C", repoDir, "rev-parse", "HEAD").Output()
	if err != nil {
		return "", fmt.Errorf("git rev-parse: %w", err)
	}
	return strings.TrimSpace(string(sha)), nil
}

// revertCanonicalFile checks out the file's content from the supplied
// commit SHA into the working tree, then commits the revert as a new
// commit with the daemon as author. Returns the new revert-commit SHA.
//
// Caller is responsible for ensuring the SHA exists in the per-dir git
// history; rev-parse + checkout will surface errors for invalid SHAs.
func revertCanonicalFile(root, relPath, sha, author, message string) (string, error) {
	topDir := canonicalTopDir(relPath)
	if topDir == "" {
		return "", errors.New("revert: root-level file has no per-dir git")
	}
	repoDir := filepath.Join(root, topDir)
	repoRel := strings.TrimPrefix(relPath, topDir+"/")
	// Restore the file to the requested SHA's content.
	cmd := exec.Command("git", "-C", repoDir, "checkout", sha, "--", repoRel)
	if out, err := cmd.CombinedOutput(); err != nil {
		return "", fmt.Errorf("git checkout %s: %w (output: %s)", sha, err, string(out))
	}
	// Commit the revert.
	return commitCanonicalChange(root, relPath, author, message)
}
