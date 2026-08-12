# Dogfood queue — make bot-hq observable from inside itself

**This document is the task.** Work it top to bottom. Everything here came out of
running bot-hq on its own codebase on 2026-08-12/13 and watching what it could
not tell us.

Read first, in this order:
1. `cl_index_search(project="bot-hq")` — then `conventions.md`, which carries the
   gates, the migration rules, and the verification grant that lets EYES run the
   suite here.
2. `docs/plans/2026-08-11-rc3-decisions.md` — **D1–D15**, binding. Check before
   reopening anything.
3. `docs/plans/2026-08-12-rc3-reframe-contract.md` — rc3 moves the SOURCE of a
   behaviour without changing the behaviour.

Full reasoning for every item below, with the evidence that motivated it, is in
the Context Library at
`projects/bot-hq/improvements-2026-08-12-visibility-and-verification.md` (P1–P8).
Read it before starting an item; it explains *why*, and this document only says
*what*.

## The theme

rc3 made the **runtime** visible: every delivery is a row, cursors show who read
what, halts surface. That worked — 1,145 delivery rows across two production
sessions, none withheld.

But every defect that cost real time lived **upstream of the session** and was
invisible to all of it: capability checkboxes that did nothing, a prompt asserting
an enforcement that was not wired, a context limit nobody could see coming. Each
item below closes one of those blind spots.

## Order, and why

Do them in this order. P1 first because it makes the others checkable.

---

### 1. P1 — Make the composed system prompt viewable per participant

**Problem.** A spawned agent receives ~48 KB of standing instruction assembled
from six layers, **appended to claude-code's own system prompt rather than
replacing it**, and nobody — user or agent — can see the result. Every "the
prompt claims an enforcement that does not exist" defect is invisible by
construction. It also closes a loop the Roles tab opened: the user edits role
prose and has no way to see it in context.

**It already exists on disk.** `spawn_agent_for` writes it to
`<agent>-system-prompt.txt` in the session's MCP temp dir before launching. The
work is surfacing it, not building it.

**Done when:** a session participant's composed prompt is viewable in the UI,
attributed to that participant, and the view survives the agent respawning. If
the file is gone (session closed), say so rather than rendering blank.

**Watch for:** the prompt contains the model's auth-adjacent env context in some
paths — check before rendering it into a panel the user might screenshot.

---

### 2. P2 — Surface capability refusals as visible rows

**Problem.** When the gate refuses a tool call, the caller is told and nobody
else. So a gate that is **silently open** and a gate that is **never exercised**
look identical. Capability enforcement was decorative for weeks and no session
would have shown it.

**The cheap version:** the refusal already builds its message from
`capability_prompt::phrasing(cap).deny` in `signaling/jsonrpc.rs`. Route that to
the channel as a system row as well as to the caller.

**Done when:** a refused tool call leaves a row naming the participant, the tool,
and the capability it lacked; and a test pins that the row is written — not just
that the refusal is returned.

**Do not** make the row block or halt anything. It is a record.

---

### 3. P7 — Persist context usage

**Problem.** `ContextUsage` (used tokens + window, from claude-code's `result`
event) is forwarded to the UI by `tauri_events/bridge_subscriber.rs` and **never
written down**. On 2026-08-12 a participant died mid-session with
`Prompt is too long` and there is no record of what its meter showed beforehand —
so the failure cannot be diagnosed after the fact, only watched live.

**Done when:** each `result` event's context usage is persisted against the
session and participant, and a closed session can still answer "what was its
context doing before it died".

**Open question to answer with evidence, not assumption:** the model's configured
`context_window` is 1,000,000 (correct — DeepSeek V4-Pro is a 1M model), yet the
limit was hit. Establish whether the `contextWindow` field arrives at all through
that gateway, and if it does not, whether the meter should fall back to
`models.context_window`. **Measure it — do not reason about it.** Persisting the
raw operands is what makes this answerable next time.

---

### 4. P6 — Push the Context Library, safely

**Problem.** `~/.bot-hq/library` now has a private remote (`bot-hq-dev-data`) and
**nothing pushes to it**. Agents commit as they work, so it drifts from the first
session onward — a snapshot, not a backup.

**The order matters here.** Before any automatic push, add a **pre-push secret
scan**. A production database credential file sat committed in that repo for 153
commits and was caught only because someone looked before the first push.
`.gitignore` stops accidents; it does not stop an agent running `git add -f`.

**Done when:** a push path exists AND a push carrying a credential-shaped blob is
refused with a message naming the file. Test the refusal, not just the push.

---

### 5. P4 — Context Library staleness detection

**Problem, and it is the structural one.** bot-hq's main advantage over plain
claude-code is the CL. The CL is maintained by bot-hq sessions. On 2026-08-12 an
audit found ~57 stale agent-name references and a whole learning describing the
native connector — deleted that day — in confident present tense. The loop that
keeps the advantage sharp did not hold, and an outsider caught it.

**This one is mechanically detectable.** A CL entry naming a symbol, file or flag
that no longer exists in the repo is a grep away: `strip_claude_code_tool_inventory`,
`may_run_native`, `models.native`, `AgentRole` were all named in CL prose and none
exist in the tree.

**Done when:** something reports CL claims that name code which is gone. A report
is enough — do NOT auto-edit the library.

**Design constraint, from D15:** any automated CL work must permit silence. An
agent with nothing to say, prompted to write, produces plausible filler, and this
layer exists so future sessions orient from it *instead of* re-reading the code.
Invented content is not noise to prune later; it is fabricated knowledge that
compounds.

---

## How to work

- **Small, testable chunks.** Compile and test after each. The five gates and
  their required ORDER are in `conventions.md` — the order is load-bearing, not
  style.
- **Pin the wire, not the halves.** For any change spanning two functions, delete
  the line that joins them and run the suite. If it stays green, you are not
  done. This defect has shipped five times in this codebase, twice while being
  fixed.
- **Mutation-verify every test you add.** Apply the smallest change to the
  production line the test claims to catch, run only that test, confirm RED,
  restore, confirm GREEN. **Never revert with `git checkout <file>`** — copy the
  file aside first and restore from the copy.
- **Migrations are immutable once applied.** Highest is 0049. If you need a new
  one, take 0050 and say so.
- **Never `--no-verify`.** Never push the bot-hq repo without the user.
- **Update `PROGRESS.md`** with a newest-first entry when a chunk lands.

## What NOT to do

- Do not take P8/P9 (the parked-question halt). Already fixed in `b559bf7` —
  read it if a session ever seems to spin, but do not redo it.
- Do not remove the `close_session` `PARITY_HOLD`. It is a real behaviour change
  and explicitly the user's call.
- Do not rewrite `decisions.md` or `issues.md` to remove the retired agent names.
  Those are append-only historical logs; rewriting them falsifies the record.
  Both already carry a vintage header.
- Do not rename the external driver's wire fields (`brian_model_id`,
  `set_agent_config(brian|rain)`). Deliberately kept — renaming breaks a driver
  already written. Single edit point is `external_jsonrpc::wire` if the user ever
  asks.

## If you finish early

Ask the user before starting anything not on this list. The remaining known items
are P3 (EYES' review method — the user owns that prose) and the two `on_demand`
questions in P8's write-up, both of which need a decision before code.
