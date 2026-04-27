package mcp

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
)

// preCommitHookScript blocks commits when origin/main has advanced past
// the worktree's HEAD. Catches the bug-#4 era stale-base merge pathology:
// coder spawns from main@A, main advances to main@B, coder commits stale-base
// then merges → wholesale revert. Coder must rebase before committing.
//
// `git fetch origin || true` so an unreachable remote (CI sandbox, offline)
// degrades gracefully — the ancestor check uses whatever ref state is local.
const preCommitHookScript = `#!/usr/bin/env bash
set -e
git fetch origin --quiet 2>/dev/null || true
if git rev-parse --verify origin/main >/dev/null 2>&1; then
  if ! git merge-base --is-ancestor origin/main HEAD 2>/dev/null; then
    echo "ERROR: worktree base is stale relative to origin/main." >&2
    echo "  origin/main has advanced past your base. Rebase before committing:" >&2
    echo "    git fetch origin && git rebase origin/main" >&2
    exit 1
  fi
fi
exit 0
`

// installWorktreeFreshnessHook installs the pre-commit freshness gate into
// the given worktree. Per-worktree scope via core.hooksPath (worktree config),
// not the shared $GIT_COMMON_DIR/hooks dir — keeps the gate isolated to coder
// worktrees without modifying the user's main-repo hooks.
//
// extensions.worktreeConfig must be enabled in shared config before
// `git config --worktree` writes; the enable is idempotent.
func installWorktreeFreshnessHook(ctx context.Context, worktreePath string) error {
	hooksDir := filepath.Join(worktreePath, ".bot-hq-hooks")
	if err := os.MkdirAll(hooksDir, 0o755); err != nil {
		return fmt.Errorf("mkdir hooks: %w", err)
	}
	hookPath := filepath.Join(hooksDir, "pre-commit")
	if err := os.WriteFile(hookPath, []byte(preCommitHookScript), 0o755); err != nil {
		return fmt.Errorf("write hook: %w", err)
	}
	if out, err := exec.CommandContext(ctx, "git", "-C", worktreePath, "config", "extensions.worktreeConfig", "true").CombinedOutput(); err != nil {
		return fmt.Errorf("enable worktreeConfig: %w: %s", err, out)
	}
	if out, err := exec.CommandContext(ctx, "git", "-C", worktreePath, "config", "--worktree", "core.hooksPath", hooksDir).CombinedOutput(); err != nil {
		return fmt.Errorf("set core.hooksPath: %w: %s", err, out)
	}
	return nil
}
