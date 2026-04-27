# Dispatch patterns (H-21)

**Status:** convention (Phase H slice 4 H-21)
**Origin:** Slice 1 verify pass (item 1, msgs 3250-3251) — `exec.Command` arg arrays, `send-keys -l` literal mode, and gemma allowlist gate were already-hardened in prior phase but never explicitly codified. Slice 4 H-21 ratchets the hardening into a written doctrine so future contributors don't re-introduce the failure modes.
**Codified in:** slice 4 design `docs/plans/2026-04-27-phase-h-slice-4-design.md` §H-21

## Rule

> **H-21 (process):** All shell-out / dispatch surfaces in bot-hq must follow the patterns below. They are not optional; they exist because the unsafe alternatives are easy to reach for and produce hard-to-diagnose bugs (command injection, partial-line dispatch, allowlist bypass). When a new dispatch surface is added, name the pattern it follows in the commit message.

## Pattern 1 — `exec.Command` arg arrays, never string concat

**Do:**

```go
exec.Command("git", "diff", "--cached", "--", filePath)
```

**Don't:**

```go
exec.Command("sh", "-c", "git diff --cached -- " + filePath) // command injection if filePath has spaces or shell metacharacters
exec.Command("git diff --cached -- " + filePath)             // doesn't even work; exec.Command name must be a single binary
```

**Why:** arg arrays bypass the shell entirely — the OS executes the binary with the arg vector, so spaces, quotes, and metacharacters in args are literal data. String concat into `sh -c` re-enables shell parsing on caller-controlled input, which is the canonical command-injection footgun.

**When you genuinely need a shell pipeline** (e.g. `cmd | grep | wc -l`): write the args to a temp script with hard-coded literal commands, OR break the pipeline into multiple `exec.Command` calls with `os.Pipe()` between them. Don't synthesize shell strings from caller data.

## Pattern 2 — Tmux `send-keys -l` literal mode for content

**Do:**

```go
exec.Command("tmux", "send-keys", "-l", "-t", target, content)
```

**Don't:**

```go
exec.Command("tmux", "send-keys", "-t", target, content) // raw mode interprets content as keybindings; "C-c", "Enter", literal "$" all collide
```

**Why:** without `-l` (literal), tmux interprets the content argument as a sequence of key names — `C-c` becomes ctrl+c, `Enter` becomes a newline keypress, `$` may interpolate. For multi-line agent prompts and message bodies this corrupts content unpredictably. `-l` mode dispatches the content as raw bytes; the receiving agent sees exactly what was sent.

**Submitting after content:** `send-keys -l` sends bytes only, no Enter. Follow with a separate `send-keys Enter` if you need to submit the input field.

## Pattern 3 — Gemma allowlist gate for shell-out commands

Gemma (Emma's analyze-class commands) gates external command execution through a static allowlist defined in `internal/gemma/gemma.go` (`allowedCommands` constant at line 74; `IsCommandAllowed` enforcement at line 299). Any new shell-out from a gemma surface must:

1. Add the binary name to the allowlist (compile-time constant; no dynamic appends)
2. Use `exec.Command` arg arrays (Pattern 1)
3. Surface command rejections via the standard error path so observed behavior matches the allowlist

**Do not** add an "escape hatch" that bypasses the allowlist for "trusted" callers. The allowlist is the trust boundary; expanding it requires a deliberate code edit + review. Per `feedback_bcc_disguise_and_origin_rules.md`, only Brian may propose allowlist changes (Rain is read-only on this surface).

## Pattern 4 — Sneak-resistant guards read pre-edit state

When a guard checks "should this commit be allowed?", read the **HEAD blob** of the file under inspection — not the working-tree version. The working-tree version is whatever the committer just wrote, including any contradictory `Status:` flip or other metadata change made in the same commit; the HEAD version is what the guard's policy was authored against.

**Example (slice 4 C1, H-6 closed-arc append-only hook):** the hook checks `git show HEAD:<file> | grep "Status: closed"` rather than `head -10 <file> | grep "Status: closed"`. A coder cannot sneak past the gate by flipping `Status: closed` → `Status: open` in the same commit they edit the body — the HEAD version still says closed, so the gate fires.

**Generalization:** any pre-commit / pre-merge check whose policy depends on file metadata must lock the policy to a pre-edit reference (HEAD, base-branch tip, or a content-addressed snapshot). Locking to working-tree state is equivalent to letting the committer rewrite the policy alongside the violation.

## Anti-patterns (named, rejected)

| Pattern | Why it fails |
|---|---|
| **String-concat into `sh -c`** | Command injection on any caller-influenced data; no realistic input-sanitization story for shell metacharacters. Drop in favor of arg arrays + script files. |
| **Raw `tmux send-keys` for prompt bodies** | Silent content corruption (key-name interpretation). The bug shows up as "the agent received a different message than what was sent" with no traceable cause. |
| **Dynamic-appended gemma allowlist** | Defeats the purpose of having an allowlist; turns the "trust boundary" into a debug surface. If a new command is needed, edit the constant. |
| **Working-tree-read guards** | Sneak-around path: committer edits the policy file alongside the violation. Always read pre-edit state (HEAD blob). |

## Refs

- `internal/mcp/worktree_hook.go` — applies Pattern 4 (HEAD-blob read for closed-arc gate)
- `internal/gemma/gemma.go` (`allowedCommands` + `IsCommandAllowed`) — Pattern 3 enforcement surface
- `internal/mcp/tools.go` — `hub_spawn` + tmux `send-keys -l` callsites (Pattern 1 + Pattern 2 example)
- `feedback_bcc_disguise_and_origin_rules.md` — allowlist-change authority
- Slice 1 verify pass (msgs 3250-3251) — original hardening discovery
