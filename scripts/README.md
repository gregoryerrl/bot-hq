# bot-hq scripts

Operational tooling that ships from the bot-hq repo for use in client-repo workflows.

## disguise-scan.sh

Pre-commit grep scan for bot-hq leaks in client-repo planning artifacts.

### Why

Bot-hq agents (Brian, Rain, Emma) regularly write planning artifacts (`docs/plans/*.md`, audit summaries, retro reviews) into client repos. These artifacts can leak agent names, hub-message-ids, internal jargon (BRAIN/HANDS/EYES, DISC v2, greenflag, trio), and other bot-hq-internal context that must not be visible to client teams.

The scan catches the leak class before it reaches commit. It uses the canonical regex from the bot-hq disguise-rule feedback memory and is safe to run on any file or directory.

### Scope

- **Run against:** client-repo planning artifacts (`docs/plans/*.md`, audit docs, retro reviews, sanitized-share artifacts).
- **Exempt:** bot-hq internal docs (`docs/arcs/*.md`, internal design notes) — agent-name and cycle-id refs are appropriate there. **Do not install this hook in the bot-hq repo itself.**

### Usage

```bash
# Scan a single file
./scripts/disguise-scan.sh docs/plans/audit-2026-04-28.md

# Scan a directory tree
./scripts/disguise-scan.sh docs/

# Scan all files staged for the next commit (typical pre-commit use)
./scripts/disguise-scan.sh --staged

# Help
./scripts/disguise-scan.sh --help
```

### Exit codes

| Code | Meaning |
| ---- | ------- |
| 0    | Clean (no matches) |
| 1    | Leak detected (offending lines printed) |
| 2    | Usage error (missing args, path not found, not a git repo for `--staged`) |

### Output

On leak: `disguise-scan: leak detected` to stderr, then `file:line:content` lines to stdout.

On clean: `disguise-scan: clean` to stderr.

### Client-repo installation as a pre-commit hook

In the client repo, add a pre-commit hook that invokes the bot-hq scan:

```bash
# Example: client-repo .git/hooks/pre-commit (chmod +x)
#!/usr/bin/env bash
set -euo pipefail

BOT_HQ="${BOT_HQ:-$HOME/Projects/bot-hq}"
if [[ -x "$BOT_HQ/scripts/disguise-scan.sh" ]]; then
    "$BOT_HQ/scripts/disguise-scan.sh" --staged
fi
```

The hook runs against staged files only; unstaged work is not scanned. Clients without bot-hq checked out should keep the hook a no-op (the `-x` test fails silently and the commit proceeds).

For Husky / lint-staged setups, wire the same invocation into the existing pre-commit chain.

### Agent invocation pattern

When a bot-hq agent works on planning artifacts inside a client repo (e.g., drafting `bcc-ad-manager/docs/plans/foo.md`), invoke the scan manually before staging:

```bash
~/Projects/bot-hq/scripts/disguise-scan.sh path/to/draft.md
```

A non-zero exit means a leak was found and must be sanitized before commit. Re-run after every parallel-coder write completes (the disguise-scan feedback memory documents a race-condition where a residual leak surfaces only on a post-DONE second scan).

### Pattern source

The regex is the verbatim canonical pattern from `feedback_disguise_scaffold_scan.md` (user auto-memory), with one refinement: `\bDISC\s+v2\b` whole-token form to avoid Discord/Discovered false positives. Adjust the script if the feedback memory updates; do not fork the regex inline.

### Tests

`disguise-scan.test.sh` is a 30-case smoke test covering positive (leak) cases, negative (clean) cases, FP-class boundary cases (Discord/Discovered exclusion, RAINS suffix), and one documented FP-but-accepted trade (the `\brain\b` weather-rain match — the regex catches non-agent uses too, traded against missing real Rain leaks).

```bash
./scripts/disguise-scan.test.sh
# Results: 30 passed, 0 failed
```

Run before any change to `disguise-scan.sh` to verify behavior is preserved.
