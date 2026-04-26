# Phase G v1 Slice 1.5 Design

Status: drafting | Owner: Brian (HANDS) | Reviewer: Rain (EYES, pre-spec adversarial gate) | Greenlight: saltegge @ msg 2983 (Path C) + msg 3024 (option (a) BRAIN consensus)

## 1. Goal

Land the focus-model rewrite saltegge picked over the A/B options I had on the table in msg 2979. Rewrite collapses three loose user-surface complaints from rebuilds #7-#9 into one coherent spec and fixes the captured-pane modal cosmetic cronk surfaced during rebuild-#9 item 5 verification.

**Slice 1.5 surface (Code):**
- C1 — Path C focus model: only `tab`/`shift+tab` switches tabs; alphanumeric auto-focuses input (printable + paste); only `end` jumps to bottom.
- C2 — Scrolled-up indicator: subtle visual hint when `!followBottom` so the user knows new messages are accumulating.
- C3 — Captured-pane modal cosmetic cleanup: diagnose + polish modal render path under no-test-fix waiver folded into normal gate (saltegge picked option (a), msg 3024).

**Convention carryover:** Slice 1 item 3 (lowercase `g` auto-focus vs jump) is resolved by C1 — under Path C, lowercase `g` auto-focuses input (types literal `g`), and capital `G` is dropped as a jump key. Item 3 is NOT a separate work item; it disappears in C1's truth table.

**Out of scope for 1.5:**
- Slice 2 work (SNAP-typed schema + DB tables + arc.md per spec §3) — resumes after 1.5 rebuild gate closes.
- hub_unregister cleanup of legacy zombies (b4e5593f, gemma-agent) — cosmetic SQL chore, low priority.
- Modal `send-keys` (read-write tmux) — locked out of v1 entirely.
- Tab-bar visual restyle, color theme changes, or any styling outside the three surfaces above.

**Sequencing:** Slice 1.5 ships → flag rebuild #10 → saltegge eyeballs full focus model + indicator placement + modal polish → PASS → Slice 2 starts.

## 2. C1 — Path C focus model rewrite

### 2.1 Behavior spec (truth table)

User-surface contract per saltegge msg 2983.

**Global keys (bypass per-tab dispatch unconditionally, including when any input/editor is focused):**

| Key | Behavior |
|---|---|
| `tab` | Cycle to next tab (TabHub→TabAgents→TabSessions→TabSettings→TabHub) |
| `shift+tab` | Cycle to previous tab |
| `ctrl+c` | Quit |

**Explicit non-goal (Rain C1.a, msg 3030 + 3031):** `tab` and `shift+tab` cycle tabs unconditionally and are NOT forwardable to any textarea. Tab characters cannot be typed into the hub or settings inputs. Recoverable via paste if a literal tab is needed. Trade-off accepted: chat/settings inputs almost never need a literal tab; symmetric global tab cycling beats focus-aware fallthrough on cost (~3 LOC saved + no new "any-input-focused-on-active-tab" abstraction).

**Per-context contract:**

| Context | Key | Behavior |
|---|---|---|
| TabHub, input not focused | `end` | Jump viewport to bottom + `followBottom = true` |
| TabHub, input not focused | `esc` | No-op (textarea blur is idempotent — Rain C1.c) |
| TabHub, input not focused | printable char (`a`-`z`, `A`-`Z`, `0`-`9`, punctuation) | Auto-focus input + forward keystroke (types literal char) |
| TabHub, input not focused | bracketed paste | Auto-focus input + forward paste |
| TabHub, input not focused | `PgUp`/`PgDn`/arrows/mouse wheel | Viewport scroll; recompute `followBottom = AtBottom()` |
| TabHub, input focused | `enter` (real keystroke, not Paste) | Submit |
| TabHub, input focused | `esc` | Blur input |
| TabHub, input focused | any other (except global keys above) | Forward to textarea (numbers, `q`, `g`, `G`, `/`, `i`, etc.) |
| TabAgents, no modal | `j`/`down`, `k`/`up`, `enter` | Cursor nav + open modal (Slice 1 respin behavior, unchanged) |
| TabAgents, no modal | any other (except global keys) | No-op — silently dropped (Rain C1.b) |
| TabAgents, modal open | `r`, `f`, `esc`, scroll | Modal-internal (unchanged) |
| TabAgents, modal open | any other (except global keys) | No-op — modal swallows |
| TabSessions | session-list nav keys | Existing handlers (unchanged) |
| TabSessions | any other (except global keys) | No-op — silently dropped (Rain C1.b) |
| TabSettings, editor not active | settings-nav keys | Existing handlers (unchanged) |
| TabSettings, editor not active | any other (except global keys) | No-op — silently dropped (Rain C1.b) |
| TabSettings, editor active | `esc` | Exit editor |
| TabSettings, editor active | any other (except global keys) | Forward to settings textarea |

**Test-boundary corollary (Rain C1.b):** the "any other → no-op" rows lock that 1.5 has zero work to add cross-tab auto-focus. Tests should NOT assert that pressing a printable char on TabAgents/Sessions/Settings does anything; assert it does NOT (input not focused there).

**Dropped from prior model:**
- `case "1"`, `"2"`, `"3"`, `"4"` in `app.go:234-241` — number keys no longer jump-to-tab. Numbers auto-focus and type into hub input. Tab cycling via `tab`/`shift+tab` remains.
- `case "q"` in `app.go:228` (currently coupled with `ctrl+c` for quit) — `q` becomes printable, types into input. Quit is `ctrl+c` only.
- `case "/", "i":` in `hub_tab.go:133` — vim-style focus shortcuts removed. `/` and `i` are printable, type literally.
- `case "G", "end":` in `hub_tab.go:136` — narrowed to `case "end":`. Capital `G` becomes printable (auto-focus + types `G`). Lowercase `g` was already printable in the prior model; behavior unchanged for `g`, changed for `G`.

### 2.2 File-level changes

**`internal/ui/app.go` — KeyMsg switch (lines 227-262):**

```go
// Before:
switch msg.String() {
case "ctrl+c", "q":
    return a, tea.Quit
case "tab":
    a.activeTab = Tab((int(a.activeTab) + 1) % len(tabNames))
case "shift+tab":
    a.activeTab = Tab((int(a.activeTab) + len(tabNames) - 1) % len(tabNames))
case "1": a.activeTab = TabHub
case "2": a.activeTab = TabAgents
case "3": a.activeTab = TabSessions
case "4": a.activeTab = TabSettings
default: ...

// After:
switch msg.String() {
case "ctrl+c":
    return a, tea.Quit
case "tab":
    a.activeTab = Tab((int(a.activeTab) + 1) % len(tabNames))
case "shift+tab":
    a.activeTab = Tab((int(a.activeTab) + len(tabNames) - 1) % len(tabNames))
default:
    // ...existing per-tab forwarding (TabHub/TabSessions/TabAgents/TabSettings)
}
```

Net: -8 LOC (drop 4 number cases + collapse `q` from quit case).

**`internal/ui/hub_tab.go` — KeyMsg unfocused branch (lines 130-163):**

```go
// Before:
key := msg.String()
switch key {
case "/", "i":
    h.focused = true
    cmds = append(cmds, h.input.Focus())
case "G", "end":
    h.viewport.GotoBottom()
    h.followBottom = true
default: ... // printable + paste auto-focus

// After:
key := msg.String()
switch key {
case "end":
    h.viewport.GotoBottom()
    h.followBottom = true
default: ... // printable + paste auto-focus (unchanged)
}
```

Net: -4 LOC (drop `/`, `i`, `G` cases; keep `end` and the printable+paste default branch intact).

### 2.3 Test surface

`internal/ui/app_test.go` — extend dispatch tests from Slice 1 respin (3 existing tests stay):

- `TestAppNumberKeysAutoFocusHubInput`: send `1` while TabHub + unfocused → `hubTab.focused == true`, input value contains `"1"`. Repeat for `2`, `3`, `4`. Assert `activeTab` unchanged.
- `TestAppQAutoFocusesHubInput`: send `q` while TabHub + unfocused → `hubTab.focused == true`, input contains `"q"`, **no quit**. Assert app still alive (no `tea.Quit` cmd returned).
- `TestAppCtrlCStillQuits`: send `ctrl+c` → returns `tea.Quit`. Sanity check the trim didn't break quit.
- `TestAppTabCyclingUnchanged`: send `tab` four times → cycles through all four tabs and back to TabHub.

`internal/ui/hub_tab_test.go` — extend (or replace) deleted `TestHubTabLowercaseGJumpsToPresent`:

- `TestHubTabSlashAutoFocuses`: unfocused + `/` → `focused == true`, input contains `"/"` (not interpreted as focus-only command).
- `TestHubTabIAutoFocuses`: unfocused + `i` → same shape, input contains `"i"`.
- `TestHubTabCapitalGAutoFocuses`: unfocused + `G` → `focused == true`, input contains `"G"`, **viewport NOT at bottom** (no jump triggered). Counterpart: `end` still jumps.
- `TestHubTabEndJumpsToPresent`: scroll up + `end` → `followBottom == true`, `AtBottom() == true`. (Equivalent to deleted G-jump test, narrowed to `end`.)

Total new tests: 7. LOC: ~80-100 across both files.

## 3. C2 — Scrolled-up indicator

### 3.1 Render condition

Indicator renders **iff `!h.followBottom`**. When user is at the bottom (auto-follow engaged), no indicator. The moment they scroll up, indicator appears. When they hit `end` (or scroll back to bottom which auto-engages follow), indicator disappears.

### 3.2 Placement options

Saltegge picks one. Both are reversible micro-decisions; spec stays the same shape regardless.

**Option α — Footer line (between strip and input):**

```
viewport       <- message feed
separator      <- existing dividing line
strip          <- existing per-agent activity dots
indicator      <- NEW: subtle text "↓ X new messages — press end to jump"
input          <- existing command input
```

- New line in `HubTab.View()` join order. Reserves 1 additional vertical cell (textarea height already fixed at `inputRows=3`, so net layout grows by 1).
- `resize()` reserved-line count goes from 3+inputRows to 4+inputRows (one extra reserved row).
- Style: foreground = `ColorStatus` (matches strip, subtle). No bold.
- Text: `"↓ scrolled up — press end to return"` (no message-count tracker in v1; counter would need a follow-bottom-snapshot delta which is scope creep).

Trade-offs:
- (+) Always positioned in user's eye line near the input bar.
- (+) No collision with viewport content.
- (−) Costs 1 vertical row of space permanently (renders empty when `followBottom`).
- (−) Visible-but-empty footer slot may itself read as cronk.

Mitigation for the (−): render an empty styled line of width N when `followBottom`, so the layout doesn't jump on toggle. Or: collapse the row and accept a 1-line layout shift on scroll (rejected — jarring).

**Option β — Viewport overlay (bottom-right corner of viewport):**

- Render indicator as a small badge in the bottom-right of the viewport when `!followBottom`. Use lipgloss.Place with `lipgloss.Right` + `lipgloss.Bottom` to overlay on the last viewport line.
- No layout reservation. Indicator appears/disappears without affecting input-bar position.
- Style: dim background block, e.g. `Background(ColorSubtle).Foreground(ColorStatus).Padding(0, 1)`, text `"↓ end to return"`.

Trade-offs:
- (+) No layout cost; appears only when needed.
- (+) Visually anchored to where the user's scroll position is — closer to "the thing you want to scroll back to."
- (−) Overlays the last visible message line. Could obscure content briefly.
- (−) Implementation: must composite onto viewport.View() output (not just JoinVertical). Requires lipgloss.Place over the viewport string.
- (−) Charm viewport doesn't natively support overlays; we'd either post-process the string (split by newline, splice into last line) or render the indicator as a separate styled string and lipgloss.Place atop. Both work but β is meaningfully more code than α.

**Recommendation:** α. Cheaper to implement, layout stability is worth the always-reserved row, "subtle hint" framing matches saltegge's wording (msg 2983: "subtle guide on bottom"). β is cleaner *if* the always-reserved row reads cronk in practice, but α-with-empty-line should be fine since the strip line above already follows the same "always-reserved" pattern (`hub_tab.go:225-237` — strip slot is reserved even when no agents alive).

**Empirical confirmation (Rain C2, msg 3030 + 3031):** scratch test verified `lipgloss.NewStyle().Width(80).Render("")` produces an 80-byte string (no newlines) of spaces, padded to width. JoinVertical with `["line1", emptyStyled, "line3"]` yields 2 newlines = 3 visual rows. **α as drafted reserves layout correctly without space-padding workaround.** Mechanism stays as in §3.3 below.

### 3.3 File-level changes (Option α path; β annotated where it diverges)

**`internal/ui/hub_tab.go` — `View()` and `resize()`:**

```go
// resize: reserved += 1 (now 4 + inputRows instead of 3 + inputRows)
// View: insert indicator between strip and input

func (h HubTab) View() string {
    separator := ...  // unchanged
    strip := ...      // unchanged

    indicator := ""
    indicatorStyle := lipgloss.NewStyle().Width(h.width).Foreground(ColorStatus)
    if !h.followBottom {
        indicator = indicatorStyle.Render("↓ scrolled up — press end to return")
    } else {
        indicator = indicatorStyle.Render("") // reserved empty row
    }

    return lipgloss.JoinVertical(lipgloss.Left,
        h.viewport.View(),
        separator,
        strip,
        indicator,
        h.input.View(),
    )
}
```

Net: ~10 LOC (new field-less helper using existing `h.followBottom`).

For β: View() returns viewport.View() with lipgloss.Place overlay; no resize change; ~25 LOC plus a string-splicing helper for the bottom-right composition.

### 3.4 Test surface

- `TestHubTabIndicatorHiddenAtBottom`: fresh HubTab + `followBottom=true` → `View()` indicator slot is empty styled string (no "↓ scrolled up" text).
- `TestHubTabIndicatorShownWhenScrolledUp`: scroll up via PgUp → `followBottom=false` → `View()` contains "↓ scrolled up — press end to return".
- `TestHubTabIndicatorHidesAfterEndKey`: scroll up → indicator visible → press `end` → `followBottom=true` → indicator hidden.

3 new tests, ~40 LOC.

## 4. C3 — Captured-pane modal cosmetic cleanup

### 4.1 Diagnosis pass — candidate cronkies

Saltegge's verdict (msg 3008): "ui of captured pane is a little cronky". Specifics not provided. Per BRAIN consensus (msg 3015 + 3017) we declined the inline waiver and folded into normal gate. Below is an enumerated diagnosis of plausible cronkies based on read of `agents_pane_modal.go`. Saltegge confirms which apply; we fix only the confirmed set.

**Candidate 1 — Title-row jitter on follow toggle (`agents_pane_modal.go:133-135`):**

```go
title := fmt.Sprintf("tmux:%s  [r] refresh  [f] follow:%s  [esc] close",
    m.target, onOff(m.autoFollow))
```

The `follow:on` ↔ `follow:off` toggle changes the title-row character count by 1. On a width-constrained terminal this can shift the trailing `[esc] close` hint horizontally between toggles. Cronk shape: jitter.

**Fix proposal:** Pad the toggle to fixed width, e.g. `follow:on ` and `follow:off`. Or move follow state to footer (cleaner separation: title = static identifier, footer = dynamic state).

**Candidate 2 — Title-row clutter (`agents_pane_modal.go:133-135`):**

The title row packs four pieces of info on one line: target name + 3 keybinding hints. On narrower terminals this wraps or truncates. Cronk shape: dense.

**Fix proposal:** Move keybinding hints to footer (where scrollback hint already lives). Title row keeps only `tmux:<target>` + maybe `(autoFollow on)` if active. Footer becomes the help line.

**Candidate 3 — Footer split brain (`agents_pane_modal.go:139-148`):**

```go
if m.lastErr != nil {
    footer = "error: ..." (red)
} else {
    footer = "(scrollback=500  ↑/↓/PgUp/PgDn to scroll)"
}
```

Footer either shows error OR scrollback hint — never both. If user wants the scroll hint while staring at an error, they can't see it. Cronk shape: information loss on error state.

**Fix proposal:** Two-line footer when error active (error line + scroll hint). Or: show error inline at top of viewport content with a marker prefix; keep footer stable.

**Candidate 4a — Inner viewport HEIGHT off-by-one (`agents_pane_modal.go:67`): CONFIRMED BUG.**

```go
m.viewport.Height = h - 5
```

**Resolution methodology (Rain msg 3033):**
- (a) `git log -L 60,73:internal/ui/agents_pane_modal.go` → constants introduced in single commit `b431a91` (Slice 1 A2 modal). Commit message describes mechanics; **does NOT explain the magic numbers**. No inline comment in code. → rationale was write-time guesswork, not load-bearing.
- (b) Empirical render at target outer = 80×24 (lipgloss.Width / lipgloss.Height of `box.Render(JoinVertical(title, body, footer))`):
  - Current `h-5` (innerH=19) → actual outer = 80×**23** (1 row short of budget)
  - Clean `h-4` (innerH=20) → actual outer = 80×**24** (fits exactly)

**Verdict:** modal under-renders by 1 row. Cronk shape = dead row immediately below the modal where the agents-tab list shows through (or just empty space if list is shorter), making the modal look like it's not centered or has unaccounted whitespace.

**Fix:** `h - 5` → `h - 4`. No comment needed — math is self-evident (1 title + 1 footer + 2 border = 4 reserved rows).

**Candidate 4b — Inner viewport WIDTH gutter (`agents_pane_modal.go:63`): DESIGN CHOICE, not a bug.**

```go
m.viewport.Width = w - 4
```

Empirical render at target outer = 80×24 (post height-fix to h-4):
- Current `w-4` (innerW=76) → actual outer = 80×24 (fits) but inner content ends at col 78 (`box.Width = w-2 = 78`), leaving a 2-col right-side gutter inside the border.
- Clean `w-2` (innerW=78) → actual outer = 80×24 (fits) and inner content fills to right border (no gutter).

**Verdict:** width math is functionally correct under both values; w-4 leaves a styling gutter, w-2 packs flush. Whether the gutter reads cronk depends on saltegge's eye.

**Fix proposal (saltegge picks):**
- **(i) Keep gutter** (`w - 4`) — saltegge says current modal felt fine width-wise, only height was cronk. Add inline comment: `// 2-col right gutter inside border for breathing room`.
- **(ii) Drop gutter** (`w - 2`) — saltegge says width felt tight or cramped, prefer flush. Update inline comment to match new math.

Default APPLY: **4a always (height bug)**. **4b → saltegge picks i or ii, defaulted to (i) keep-with-comment if no explicit preference** (preserves observed behavior, just makes the choice explicit).

**Candidate 5 — Border color saturation (`agents_pane_modal.go:151`): SPECULATIVE, OFF-DEFAULT (Rain C3.5).**

```go
box := lipgloss.NewStyle().
    Border(lipgloss.RoundedBorder()).
    BorderForeground(ColorClive)
```

`ColorClive` is bright cyan. Against a darker terminal background the rounded border may be visually heavy for what's a *capture* (read-only) modal. Hypothesized cronk shape: visual weight mismatch.

**Confidence: low.** This is Brian's hypothesis with no evidence. Saltegge said "cronky" not "wrong color." Risk: fix lands, saltegge says "cyan was fine." Marked **off-default APPLY set** — only fix if saltegge explicitly picks candidate 5 in §7 Q2 by mentioning color, border weight, or saturation. Vague "fix candidates 1, 3" does NOT include 5.

**Fix proposal (only if saltegge confirms):** Use `ColorStatus` (subtle gray) for the modal border. Reserve bright colors for active-action elements (refresh, follow toggle). Or use a dimmed variant of `ColorClive`.

### 4.2 Fix surface (after saltegge confirms)

Saltegge's pass: enumerate which candidates apply (1 or more), mark NOT-APPLY for any false-positives, optionally surface a cronk we missed. Then I implement only the confirmed set in a single commit. Diff stays surgical.

**File:** `internal/ui/agents_pane_modal.go` only. Estimated LOC: 15-30 depending on which candidates apply (Candidate 2 — restructure to title+footer split — is the largest at ~15 LOC; others are ≤5 each).

### 4.3 Test surface (~1-2 cosmetic snapshot tests, per BRAIN consensus)

- `TestPaneModalViewSnapshotIdle`: instantiate modal, set fixed size (e.g. 80x24), `Refresh()` with stubbed capture returning a known string, capture `View()` output, compare against a golden string. Locks the rendered shape.
- `TestPaneModalViewSnapshotErrorState`: same shape but stub capture returning an error → `View()` shows error line. Locks the error-state rendering separately so future fix to Candidate 3 (error+hint coexistence) is gateable.

Snapshot tests use existing test scaffold (no new test infra). Goldens stored inline as Go raw strings. ~30-40 LOC for both tests + goldens.

**Why snapshot tests despite "cosmetic":** Per BRAIN (msg 3015): cosmetic ≠ unconstrained. The fix surface is small and the regression risk is "fix borders, accidentally break the title row" — exactly what snapshot catches in 5 seconds and human eyeball misses in passing. Cost ~10 LOC, half-life is years.

## 5. Architecture notes

**Scope walls:**
- C1 (focus model) edits app.go + hub_tab.go (key dispatch). Does NOT touch modal, agents tab cursor, sessions, settings.
- C2 (indicator) edits hub_tab.go only (View + resize). Does NOT touch dispatch, modal, agents tab.
- C3 (modal cosmetic) edits agents_pane_modal.go only. Does NOT touch dispatch, hub_tab, agents_tab cursor logic.

Three independent surfaces, three commits, single-branch Slice 1.5. Each commit is reviewable in isolation. Combined diff: ~150-200 LOC across 3 files + 3 test files.

**Reversibility:**
- Indicator placement (α/β) is a single-function swap in `View()`. Choosing wrong is recoverable in <30 LOC.
- Modal cosmetic candidates each independently revertible by commit.
- Path C focus-model rewrite is the highest-cost-to-revert (user retraining), but matches saltegge's explicit pick and ships behind a rebuild gate.

**Test boundary:**
- C1 dispatch tests live in `app_test.go` (App.Update integration), `hub_tab_test.go` (HubTab.Update unit). Same split as Slice 1 respin.
- C2 indicator tests in `hub_tab_test.go` (View output assertion).
- C3 snapshot tests in `agents_pane_modal_test.go` (existing file, extend).

## 6. Sequencing

1. Rain pre-spec adversarial gate (C/P/A framework — C1-C4 / P1-P3 / A1-A4 — same as Phase G v1 spec gate).
2. Saltegge resolves: indicator placement (α vs β) + modal cronk candidate confirms (1-5 subset, candidate 4a = always APPLY per Rain C3.4 empirical).
3. Brian implements on `brian/phase-g-v1-slice-1.5` branch. Three commits: C1 (focus model), C2 (indicator), C3 (modal cosmetic).
4. Brian runs full test suite + go vet locally. Push.
5. Rain diff gate (greenflag-final until rebuild authority).
6. Brian merges + pushes origin + flags rebuild #10.
7. Saltegge eyeball after rebuild: focus model, indicator visibility/placement, modal polish. Rain SQL belt (item 6 carryover, trivial since rebuild_gen layer untouched).
8. PASS → Slice 1.5 closed → Slice 2 (SNAP-typed schema + DB tables + arc.md) starts. FAIL → partial-failure rollback per §6.1.

### 6.1 Rebuild #10 partial-failure rollback (Rain A4)

Three independent commits land together; rebuild #10 gates the combined surface. Per-surface rollback if any partial fails:

**C1 — Path C focus model fails** (e.g. tab cycling broken, printable auto-focus regression, accidental key-handler removal):
- Worst-case shape: requires full design loop (truth-table revision, new test surface, re-gate). User retraining cost is sunk regardless.
- Rollback path: branch hotfix off `main` reverting commit C1 only, push, rebuild #11 with C2+C3 retained, re-gate against the C1-revert + amended-design loop.
- Triage: distinguish "spec was wrong" (full respin) vs "implementation diverged from spec" (surgical hotfix). Test surface from §2.3 should catch implementation divergence; spec failure surfaces as user complaint matching truth-table edge case.

**C2 — Indicator fails** (e.g. chosen placement reads cronk in practice, indicator never shows or never hides, layout jumps despite α reservation):
- Surgical hotfix shape. ≤30 LOC delta to swap α↔β or revert to no-indicator baseline.
- Rollback path: branch hotfix off `main` reverting commit C2 only, push, rebuild #11. Saltegge re-picks placement if α/β swap is the call.
- Worst case: drop indicator entirely from 1.5, revisit in Slice 1.75 once saltegge has lived without it for a rebuild.

**C3 — Modal cosmetic fails** (e.g. h-4 fix breaks rendering on a specific terminal, candidate fix surfaces a regression saltegge didn't anticipate):
- Surgical hotfix shape. ≤15 LOC delta per candidate; revertible per-candidate since each is an independent edit.
- Rollback path: branch hotfix off `main` reverting individual candidate fixes within commit C3, push, rebuild #11. Snapshot tests from §4.3 should catch the divergence pre-merge.

**Cross-failure (multiple surfaces fail simultaneously):** revert the entire 1.5 merge commit, branch hotfix off `main` at the pre-1.5 SHA (`f0ec62d`), push, rebuild #11 to the pre-1.5 baseline. Then re-spin 1.5 from scratch with full-doc revision. Worst-case scenario; expected probability low given pre-spec gate + diff gate + per-surface independence.

**Diff gate signal (Rain on rebuild #10):** if any surface fails, Brian flags + Rain diff-gates the hotfix branch. No respin authority on Rain side; spec-level changes loop back to pre-spec gate.

## 7. Open questions for saltegge

**Q1 — Indicator placement: α (footer line) or β (viewport overlay)?**
Brian recommends α: cost, layout stability (verified empirically per §3.2 + Rain C2), and saltegge's "subtle guide on bottom" wording (msg 2983). β is the alternative if α reads cronk in practice. **Unblockable by gate gaps — saltegge can pick now.**

**Q2 — Modal cronkies — which candidates apply?** Mark each as APPLY / NOT-APPLY:
- **Candidate 1** (title-row jitter on `follow:on`↔`follow:off`)
- **Candidate 2** (title-row clutter, dense one-liner)
- **Candidate 3** (footer split-brain, error/scroll-hint mutex)
- **Candidate 4a** (HEIGHT off-by-one — empirically confirmed bug, **default APPLY**, no opt-out unless saltegge prefers buggy behavior for some reason)
- **Candidate 4b** (WIDTH gutter — saltegge picks (i) keep gutter / (ii) drop gutter; default (i) keep with comment)
- **Candidate 5** (border color saturation — speculative, **off-default APPLY** per Rain C3.5; only fixed if saltegge mentions color/border-weight/saturation explicitly)

Option: give one-word descriptor ("title jitter", "cramped help row", "weird gap below modal") and Brian fits to candidates. **Affected by gate gaps — recommend wait until Rain re-gates the amended candidate list (4a/4b split + 5 confidence flag landed in this revision).**

**Q3 — Indicator text — "↓ scrolled up — press end to return" or alternative?**
Open to user-facing copy edits. **Unblockable by gate gaps — saltegge can pick now.**

**Q4 — `ctrl+c` only for quit, OR also `ctrl+q`?**
Default is `ctrl+c` only per Path C. Some users prefer a no-conflict-with-copy quit binding; `ctrl+q` is cheap to add if wanted. **Unblockable by gate gaps — saltegge can pick now.**

**§7 unblock subset:** Q1, Q3, Q4 are answerable now (independent of Rain re-gate on amendments). Q2 holds for re-gate PASS so saltegge picks against the amended candidate list (4a/4b split, 5 confidence flag).

## 8. Locked decisions (Rain §8 picks, msg 3030)

These are no longer open — baked into spec as binding decisions per Rain BRAIN consensus:

- **Snapshot test goldens:** inline raw strings in `agents_pane_modal_test.go`. Threshold for `testdata/` (>100 LOC, ≥3 distinct goldens, CI normalization) does not apply at ~30-40 LOC for 2 goldens. Inline keeps tests self-contained. P4 micro-note: add inline comment near goldens documenting that lipgloss version bumps may break goldens with no behavior regression — future maintainer doesn't panic.
- **Branch shape:** **single branch** `brian/phase-g-v1-slice-1.5` with 3 commits (C1, C2, C3). Independent surfaces but interlocked rebuild gate. Three separate branches = three rebuild cycles, rejected on cost.
- **Filter assay duration:** keep legacy zombies (b4e5593f, gemma-agent at gen=0) as live filter assay through rebuild #10. Third datapoint valuable since #10 exercises agents-tab nav under post-Slice-1 conditions. `hub_unregister` cleanup deferred to cosmetic post-1.5 chore.

---

End of Slice 1.5 design — Revision 2 (post Rain msg 3030 + 3033 gate). Awaiting Rain delta re-gate.

## Changelog vs Revision 1

- **§2.1 truth table** — restructured into "Global keys" (tab/shift+tab/ctrl+c, unconditional) + "Per-context contract." Locked α resolution of C1.a (tab cycling overrides typing, with explicit non-goal). Added C1.b silence rows for TabAgents/Sessions/Settings. Added C1.c esc-unfocused no-op row.
- **§3.2 indicator** — added empirical confirmation footer noting C2 verified via scratch test (lipgloss `Render("")` produces width-padded space-row, JoinVertical preserves layout).
- **§4.1 candidate 4** — split into 4a (HEIGHT, confirmed bug via git-blame + empirical render at 80×24) + 4b (WIDTH, design choice with saltegge i/ii pick). Resolution methodology documented per Rain msg 3033.
- **§4.1 candidate 5** — marked speculative, off-default APPLY set (Rain C3.5).
- **§6 sequencing** — added §6.1 partial-failure rollback per surface (Rain A4).
- **§7 saltegge questions** — restructured to Q1-Q4 with explicit unblockable subset (Q1/Q3/Q4 unblockable now; Q2 holds for re-gate PASS).
- **§8** — converted from open questions to locked decisions (Rain §8 picks baked in).
