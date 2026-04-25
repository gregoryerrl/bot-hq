# Methodology Notes

General lessons accumulated across Phase E and the bug-fix bundles that
followed (#4, #2/#3, #1). Each entry names a recurring failure mode + the
discipline that prevents it. Cite this doc in future bundle specs and
reviews where the same shape applies.

## 1. Shared-utility bug class

When two reported bugs surface different symptoms but root in the same
absent abstraction, treat as one investigation surface, separate commits.

**Canonical example:** Bug #2 (`hub_spawn` empty `❯` after spawn) and
Bug #3 (`claude_message` false-busy on at-prompt panes) appeared as
distinct symptoms but both rooted in the missing `tmux.WaitForPrompt`
utility. Bug #2 was a brittle TIME gate (`time.Sleep(3s)`) where a
STATE gate was needed. Bug #3 was a brittle STATE check (last-line
heuristic) targeting the wrong line. Both needed the same primitive:
"scan the pane for the input prompt anchor; return when found or after
a deadline."

**Discipline:** before splitting a multi-bug investigation into separate
commits, probe whether they share a missing primitive. If yes, build the
primitive in one commit, fix each bug in its own follow-up commit
against that primitive. Keeps surface small; each fix is reviewable on
its own merits without re-arguing the primitive.

## 2. Audit metric reframe — gate calibration

Pre-merge gate metrics calibrated against an *expected* baseline must be
re-validated against the *actual* baseline before applying the threshold.
If actual differs from expected (e.g. self-imposed discipline already
shifted practice), the gate loses interpretive power.

**Canonical example:** the Bridge bundle (bug #1) audit measured HIT% at
12% recent / 28% combined. The original threshold (≥80% HIT → "rule
fits, merge"; ≤50% HIT → "wording too strict, iterate") confounded two
distinct states under low HIT%: (a) rule too strict, vs (b) baseline
already aligned with rule. The audit's MISSes were genuinely dual-
audience or user-facing under the new rule, distinguishing case (b)
from case (a). Reframe accepted: "Bundle merges if rule fits *current
practice OR future intent*. HIT% measures the gap. Low HIT% (<30%) =
rule already approximately followed → merge to lock the discipline."

**Discipline:** when running a pre-merge metric, validate the metric's
calibration assumptions against actual baseline before applying the
threshold. If they diverge, reframe the gate or the threshold doesn't
mean what it was designed to mean.

## 3. Empirical byte-anchor probes — probe the production data path

Empirical byte-anchor probes must hexdump the *actual data path* (stdout
of the tool capturing data in production), not surrogate text containing
the same visual character. U+0020 (regular space) and U+00A0 (NBSP)
render identically in terminals/editors and cannot be distinguished
without raw bytes from the production path.

**Canonical example:** Bug-2-3 commit 1 anchor was set to
`\xe2\x9d\xaf\x20` (❯ + regular space) because the original probe
hexdumped hub message text where the rule author had typed `❯ ` with a
regular space when authoring the spec — not actual `tmux capture-pane`
output of a live Claude pane. The actual byte sequence is
`\xe2\x9d\xaf\xc2\xa0` (❯ + NBSP). The visual identity of `\x20` vs
`\xa0` masked the divergence until production reality (capture-pane on
a live Claude pane) was probed directly. Caught post-commit by a fresh
hexdump of the live pane; corrected in commit 3 of the same bundle as a
distinct commit (audit trail beats history rewrite).

**Discipline:** when pinning a byte anchor for matching against tool
output, the probe target must be the actual tool's actual output. Spec
text, chat transcripts, code comments — all surrogates that may use
different bytes for the same visual character. Hexdump the production
target.

## 4. Future hardening — bug-2-3 commit 3 polish notes

Non-blocking observations from Rain's review of bug-#4 commit 3 + bug-2-3
commit 2, queued here for a future cleanup pass:

- **`cmd/bot-hq/main.go` `ReconcileCoderGhosts` error path silently
  swallows on err.** Boot proceeds either way (matches existing style),
  but a `else if err != nil { fmt.Fprintf(os.Stderr, ...) }` branch
  would surface SQL failures during boot diagnostics. ~3 LOC.
- **`TestReconcileCoderGhosts_FlipsStoppedSessionAgents` discards
  `GetAgent` errors via `_`.** In practice GetAgent only errors on
  missing-id which the test setup precludes — fine in this fixture, but
  a paranoid `t.Fatalf` on err would future-proof against schema
  changes. ~5 LOC.
- **`hub_spawn` boot-timeout error message discards captured output.**
  `at, _, err := WaitForPrompt(...)` drops the `_` capture. Including
  it in the error message ("captured at timeout: ...") would help
  diagnose what was on the pane during a boot timeout (e.g., "still
  loading MCP config", "stuck at welcome banner"). ~5 LOC.

None block; pick up in a future small bundle.

## 5. WaitForPrompt anchor implicit dependency

The `tmux.promptByteAnchor` (`❯<NBSP>`) implicitly depends on tmux
`capture-pane` preserving the trailing NBSP, which it does because NBSP
is non-whitespace from capture-pane's line-trim perspective. (This is
also why Claude Code uses NBSP rather than regular space — regular
space gets trimmed at end-of-visual-line, NBSP survives.)

If Claude's prompt rendering ever changes such that the byte after `❯`
is something other than NBSP, the anchor stops matching even though
`❯` is still on-pane. Detection: `TestWaitForPrompt_InstantDetection`
would start failing against a real Claude pane. Test-fixture behavior
already locks the regular-space-must-NOT-match invariant, so any
attempt to "fix" it by widening to regular space fails CI before
landing.

For maintenance: if Claude Code's prompt rendering changes upstream,
update both `promptByteAnchor` (to the new byte sequence) and the
"regular space must not match" test case (to whatever the new
distinction lock is). Both must move together.
