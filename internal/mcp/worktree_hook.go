package mcp

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
)

// preCommitHookScript runs two independent gates per commit attempt:
//
//  1. Stale-base gate (H-3b): blocks commits when origin/main has advanced
//     past the worktree's HEAD. Catches the bug-#4 era stale-base merge
//     pathology — coder spawns from main@A, main advances to main@B, coder
//     commits stale-base then merges → wholesale revert. Coder must rebase
//     before committing. `git fetch origin || true` so an unreachable remote
//     (CI sandbox, offline) degrades gracefully.
//
//  2. Closed-arc append-only gate (H-6): blocks any non-append-only change
//     to a `docs/arcs/*.md` file whose HEAD version has `Status: closed` in
//     the first 10 lines. Closed arcs are frozen historical records per the
//     arc-closure-discipline convention; refinements belong in the next-arc
//     pointer at next-arc-open, not retroactive in-place edits. Append-only
//     means: exactly one diff hunk, all body lines are additions, the hunk
//     attaches at the file's pre-edit end-of-file (old_count == 0 and
//     old_start == HEAD-line-count).
const preCommitHookScript = `#!/usr/bin/env bash
set -e

# Gate 1: stale-base.
git fetch origin --quiet 2>/dev/null || true
if git rev-parse --verify origin/main >/dev/null 2>&1; then
  if ! git merge-base --is-ancestor origin/main HEAD 2>/dev/null; then
    echo "ERROR: worktree base is stale relative to origin/main." >&2
    echo "  origin/main has advanced past your base. Rebase before committing:" >&2
    echo "    git fetch origin && git rebase origin/main" >&2
    exit 1
  fi
fi

# Gate 2: closed-arc append-only.
violation=0
while IFS= read -r f; do
  [ -z "$f" ] && continue
  case "$f" in
    docs/arcs/*.md) ;;
    *) continue ;;
  esac
  head_blob=$(git show "HEAD:$f" 2>/dev/null) || continue
  if ! printf '%s\n' "$head_blob" | head -10 | grep -q "^Status: closed"; then
    continue
  fi
  diff_out=$(git diff --cached --no-color -U0 -- "$f")
  hunk_count=$(printf '%s\n' "$diff_out" | grep -c '^@@' || true)
  if [ "$hunk_count" -eq 0 ]; then
    continue
  fi
  if [ "$hunk_count" -ne 1 ]; then
    echo "ERROR: closed arc '$f' has $hunk_count diff hunks; append-only requires a single end-of-file hunk." >&2
    violation=1
    continue
  fi
  if printf '%s\n' "$diff_out" | grep -E '^-[^-]' | grep -q .; then
    echo "ERROR: closed arc '$f' contains line deletions/edits; append-only required (Status: closed)." >&2
    violation=1
    continue
  fi
  header=$(printf '%s\n' "$diff_out" | grep '^@@' | head -1)
  old_meta=$(printf '%s\n' "$header" | sed -E 's/^@@ -([0-9]+)(,([0-9]+))? .*/\1 \3/')
  old_start=$(printf '%s\n' "$old_meta" | awk '{print $1}')
  old_count=$(printf '%s\n' "$old_meta" | awk '{print ($2 == "") ? 1 : $2}')
  if [ "$old_count" -ne 0 ]; then
    echo "ERROR: closed arc '$f' modifies existing lines; append-only required (Status: closed)." >&2
    violation=1
    continue
  fi
  head_lines=$(printf '%s\n' "$head_blob" | awk 'END{print NR}')
  if [ "$old_start" -ne "$head_lines" ]; then
    echo "ERROR: closed arc '$f' insertion at line $old_start, not end-of-file (HEAD has $head_lines lines); append-only required." >&2
    violation=1
    continue
  fi
done < <(git diff --cached --name-only --diff-filter=AMD)
if [ "$violation" -eq 1 ]; then
  exit 1
fi
exit 0
`

// installWorktreeHooks installs the multi-gate pre-commit hook into the
// given worktree. Per-worktree scope via core.hooksPath (worktree config),
// not the shared $GIT_COMMON_DIR/hooks dir — keeps the gate isolated to
// coder worktrees without modifying the user's main-repo hooks.
//
// Single hook script, multiple gates: core.hooksPath only honors one
// pre-commit file, and the gates are cheap + independent, so co-locating
// them in one script is simpler than juggling hook-chain machinery.
//
// extensions.worktreeConfig must be enabled in shared config before
// `git config --worktree` writes; the enable is idempotent.
func installWorktreeHooks(ctx context.Context, worktreePath string) error {
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
