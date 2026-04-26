# Phase H — Slice 1 design (Real-project safety)

**Status:** design — pending Rain diff-gate
**Arc:** `docs/arcs/phase-h.md`
**Master design:** `docs/plans/2026-04-26-phase-h-design.md`
**Branch (planned):** `brian/phase-h-slice-1` (cut from main after this design merges)

## Goal

Make it **structurally impossible** for bot-hq to push wrong-named branches, force-push without explicit user token, or take destructive actions in non-bot-hq projects. Closes the bcc-ad-manager incident class (hub msgs 999, 1009, 1736).

## Items in this slice

| Item | Description |
|---|---|
| **H-4** | Per-project rules in `~/.bot-hq/projects/<name>.yaml` (load-bearing) |
| **H-3c** | `push_requires_approval: true` default-on for non-bot-hq projects |
| **H-13** | Force-push hard-blocked unless user replies with verbatim token `force-push-greenlight: <branch>@<sha>` |
| **H-14** | `hub_spawn` hard-blocks dispatch into a project with no rules file loaded |
| **H-16** | Coder tool allowlist for non-bot-hq projects |

H-4 is **load-bearing**: H-3c/H-13/H-14/H-16 all depend on per-project rules being loadable. Implementation order = H-4 first, others after.

## Architecture

### Storage layout

```
~/.bot-hq/projects/
  bcc-ad-manager.yaml      # keyed by remote URL
  988-utah-gov.yaml
  bot-hq.yaml              # bot-hq's own rules (lenient)
  _default.yaml            # fallback for unknown projects (strictest)
```

**Filename derivation:** repo-name extracted from `git remote get-url origin` via regex `[/:]([\w-]+?)(?:\.git)?$`. E.g. `git@github.com:gregoryerrl/bcc-ad-manager.git` → `bcc-ad-manager.yaml`. Conflicts resolved by full-remote-URL hash suffix (rare; not in v1).

### Rules schema

```yaml
# ~/.bot-hq/projects/bcc-ad-manager.yaml
remote_url: "git@github.com:gregoryerrl/bcc-ad-manager.git"  # for verification at load
project_name: "bcc-ad-manager"

# Branch creation rules
branch_pattern: "^[0-9]+-[a-z0-9-]+$"           # required; coders must conform
branch_examples:
  - "346-streamline-onboarding-process"
  - "355-duplicateadjob-fails-for-lead-generation-campaigns"
branch_pattern_help: "Use [issueNo]-[title-with-dashes]; lowercase only"

# Push approval (H-3c)
push_requires_approval: true                     # default true for non-bot-hq

# Force-push (H-13)
force_push_blocked: true                         # hard-block; user token-greenlit only
force_push_token_format: "force-push-greenlight: {branch}@{sha}"

# Coder tool allowlist (H-16)
coder_tools_blocked:
  - "git push"
  - "git push --force"
  - "gh pr create"
  - "gh pr merge"
  - "gh issue close"
  - "rm -rf"
coder_tools_per_action_approval:
  - "git commit"   # require Brian/Rain approval per commit (optional, off by default)

# Optional metadata (informational only in v1)
commit_style: "imperative-mood"
require_issue_link: false
```

```yaml
# ~/.bot-hq/projects/bot-hq.yaml — lenient self-rules
remote_url: "git@github.com:gregoryerrl/bot-hq.git"
project_name: "bot-hq"
branch_pattern: "^(brian|rain|coder-[a-f0-9]+)/[a-z0-9-]+$"
push_requires_approval: false                    # bot-hq self-pushes freely
force_push_blocked: false                        # bot-hq can force-push
coder_tools_blocked: []                          # full coder access in bot-hq dir
```

```yaml
# ~/.bot-hq/projects/_default.yaml — strictest fallback (used only by H-14 bootstrap-fail message; never auto-applied)
project_name: "_default"
branch_pattern: ".*"                              # no enforcement
push_requires_approval: true                     # paranoid default
force_push_blocked: true
coder_tools_blocked:
  - "git push"
  - "git push --force"
  - "gh pr create"
  - "gh pr merge"
  - "rm -rf"
```

### Loader contract

New package: `internal/projects/`.

```go
// internal/projects/projects.go
package projects

type Rules struct {
    RemoteURL                  string   `yaml:"remote_url"`
    ProjectName                string   `yaml:"project_name"`
    BranchPattern              string   `yaml:"branch_pattern"`
    BranchExamples             []string `yaml:"branch_examples"`
    BranchPatternHelp          string   `yaml:"branch_pattern_help"`
    PushRequiresApproval       bool     `yaml:"push_requires_approval"`
    ForcePushBlocked           bool     `yaml:"force_push_blocked"`
    ForcePushTokenFormat       string   `yaml:"force_push_token_format"`
    CoderToolsBlocked          []string `yaml:"coder_tools_blocked"`
    CoderToolsPerActionApproval []string `yaml:"coder_tools_per_action_approval"`
    CommitStyle                string   `yaml:"commit_style"`
    RequireIssueLink           bool     `yaml:"require_issue_link"`
}

// LoadForProject reads ~/.bot-hq/projects/<derived_name>.yaml.
// Returns (nil, ErrNoRulesFound) if file doesn't exist — caller must bootstrap.
// Returns (nil, ErrRemoteMismatch) if file's remote_url != actual remote.
// Never auto-applies _default.yaml (used only as bootstrap-fail message template).
func LoadForProject(projectDir string) (*Rules, error)

// DeriveProjectName extracts canonical name from a remote URL.
// "git@github.com:org/foo.git" → "foo"
// "https://github.com/org/foo" → "foo"
func DeriveProjectName(remoteURL string) string

// ValidateBranchName checks name against rules.BranchPattern.
// Returns nil on match, or *ValidationError with help text.
func (r *Rules) ValidateBranchName(name string) error

// IsCoderToolBlocked returns true if the given command-line is blocked.
// Match is prefix-based on coder_tools_blocked entries.
func (r *Rules) IsCoderToolBlocked(cmdline string) bool

var (
    ErrNoRulesFound   = errors.New("no project rules file found")
    ErrRemoteMismatch = errors.New("project rules file remote_url does not match actual remote")
)
```

### Integration points

**H-14 (`hub_spawn` hard-block):** in `internal/mcp/tools.go` `hub_spawn` handler, before tmux spawn:
```go
remoteURL, err := exec.Command("git", "-C", project, "remote", "get-url", "origin").Output()
if err != nil {
    return mcp.NewToolResultError("hub_spawn requires a git project with origin remote"), nil
}
rules, rulesErr := projects.LoadForProject(project)
if rulesErr != nil {
    return mcp.NewToolResultError(fmt.Sprintf(
        "hub_spawn blocked: no rules file for project. Bootstrap required.\n\n" +
        "Run bootstrap flow: ask user to confirm rules for %s, save to ~/.bot-hq/projects/%s.yaml\n" +
        "See ~/.bot-hq/projects/_default.yaml for template.",
        projects.DeriveProjectName(string(remoteURL)),
        projects.DeriveProjectName(string(remoteURL)),
    )), nil
}
// continue with spawn, passing rules into coder preamble
```

**H-3c (push approval):** coder preamble (`hubPreamble` in `tools.go:720`) gains a section if `rules.PushRequiresApproval`:
```
PUSH POLICY: This project requires user approval before any git push.
- Do NOT run `git push` or `git push --set-upstream`.
- When push is needed, hub_send to brian: "ready to push branch X, awaiting approval".
- Wait for explicit approval before pushing.
```

**H-13 (force-push token):** wrap any push attempts with token check. Implementation can be either:
- (a) coder-side: coder MUST send `request_force_push: <branch>@<sha>` to brian, brian flags user, user replies with token, brian relays approval
- (b) hub-side: brian gets a new MCP tool `verify_force_push_token` that checks user msg history for the token before greenlighting

V1: option (a) — simpler, no new tool. Token verification = brian re-reads recent user messages for verbatim token match against the requested branch+sha.

**H-16 (coder tool allowlist):** coder preamble lists `rules.CoderToolsBlocked`. Coder is instructed to refuse those commands; if user/brian asks coder to run a blocked command, coder PMs brian with a refusal message. **No active enforcement at the bash layer** in v1 (coders run with full shell permissions); the gate is prompt-discipline. v2 candidate: actual shell-wrapper filtering.

**Bootstrap flow (friendly-fail per default):**
1. Brian attempts `hub_spawn` into unknown project → blocked with structured error
2. Brian flags Rain ("new project detected: `<name>` at `<remote>`, no rules file")
3. Rain inspects: `git -C <project> branch -r | head -20` to identify branch convention pattern
4. Rain proposes rules file content; flags user via `hub_flag` for confirmation
5. User confirms (or edits + confirms); Rain saves file to `~/.bot-hq/projects/<name>.yaml`
6. Brian retries `hub_spawn`; succeeds.

## C-series implementation order

| Commit | Items | Description | Tests |
|---|---|---|---|
| **C1** | H-4 | `internal/projects/` package: schema, loader, validators. + `~/.bot-hq/projects/_default.yaml` ships as bootstrap template. + `~/.bot-hq/projects/bot-hq.yaml` ships as self-rules. | TestDeriveProjectName, TestLoadForProjectMissingFile, TestLoadForProjectRemoteMismatch, TestValidateBranchName, TestIsCoderToolBlocked |
| **C2** | H-14 | `hub_spawn` pre-flight: git-remote-check + projects.LoadForProject. Block with structured bootstrap message on miss. | TestHubSpawnBlocksOnMissingRules, TestHubSpawnAllowsWithRules, TestHubSpawnBlocksOnRemoteMismatch |
| **C3** | H-3c + H-16 | Coder preamble extension: PUSH POLICY section + COMPLETE TOOL ALLOWLIST section per rules. (H-3c is "no push without approval"; H-16 is "no destructive ops without approval".) | TestCoderPreambleIncludesPushPolicy, TestCoderPreambleIncludesToolBlocks, TestCoderPreambleSuppressedForBotHqProject |
| **C4** | H-13 | Force-push token verification. Brian's `request_force_push` PM handler + token re-read against user message history. | TestForcePushBlockedWithoutToken, TestForcePushAllowedWithExactToken, TestForcePushBlockedWithStaleSHAToken, TestForcePushBlockedWithStaleBranchToken |
| **C5** | doc + ratchet | Add `~/.bot-hq/projects/bcc-ad-manager.yaml` + 988-utah-gov.yaml as concrete user examples. + arc.md update (in-flight → merged). + slice 1 closure entry. | (no new tests; doc-only) |

**Estimated diff:** C1 ~250 LOC + tests; C2 ~50 LOC + tests; C3 ~80 LOC + tests; C4 ~120 LOC + tests; C5 doc-only. Total ~500 LOC + ~15 test cases.

## Risk + mitigation

| Risk | Likelihood | Mitigation |
|---|---|---|
| Coder ignores prompt-level allowlist (H-16 v1 isn't enforced at shell layer) | Medium | Codify in coder preamble with explicit refusal protocol + Brian audits coder hub messages for blocked-command attempts. v2 = actual shell-wrapper. |
| User mistypes force-push token | Low | Token includes branch + sha; mistype → no match → no push. User retries. |
| Bootstrap flow user confirms wrong rules | Low-medium | Rain's inspection step shows examples from `git branch -r`; user reviews before confirming. Rules file is plain YAML, easy to edit later. |
| H-4 `_default.yaml` accidentally auto-applied to unknown project | Low | Loader hard-fails with `ErrNoRulesFound` instead of fallback; `_default.yaml` only shown in error message. |
| `git remote get-url origin` returns nothing or non-standard URL | Low | Loader handles gracefully; bootstrap message includes raw remote URL for user to specify project name manually. |

## Out of scope (v1)

- Shell-layer enforcement of H-16 (v2)
- Multi-remote projects (rare; v2)
- Per-branch rule overrides (rare; v2)
- Pre-commit hooks for branch naming (covered by H-14 dispatch-time gate; pre-commit redundant)
- Auto-creating GitHub issues (premature)
- Auto-generating branch names from issue titles (premature)

## Test plan

Unit tests per C-series above. Integration test (manual): bootstrap flow against bcc-ad-manager.

**Acceptance criteria for slice 1:**
1. `hub_spawn` into bcc-ad-manager without `~/.bot-hq/projects/bcc-ad-manager.yaml` → blocked with structured message
2. After bootstrap → `hub_spawn` succeeds, coder preamble includes branch_pattern + push policy + tool allowlist
3. Coder attempts `git push` → coder refuses (per preamble) and PMs brian
4. Force-push attempt → blocked unless verbatim token replied by user
5. `hub_spawn` into bot-hq itself → succeeds with lenient self-rules (no push approval, no force-push block)
6. All test cases per C-series pass; `go vet ./...` clean; `go build ./...` clean

## Refs

- arc: `docs/arcs/phase-h.md`
- master design: `docs/plans/2026-04-26-phase-h-design.md`
- bcc-ad-manager incident: hub msg 1009 (23-file revert), msg 1736 (PR #346/353/355 branch convention)
- dispatch code path: `internal/mcp/tools.go:568-737` (hub_spawn body)
- existing allowlist precedent: `internal/gemma/allowlist.go` (pattern reference)
