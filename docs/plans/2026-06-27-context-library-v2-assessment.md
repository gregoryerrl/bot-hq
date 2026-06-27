# Context Library v2 — assessment + recommended path

- **Date:** 2026-06-27
- **Status:** Assessment / recommendation. Not yet ratified — awaiting a scope decision.
- **Author:** Brian (HANDS), grounded in bot-hq internals.
- **Source under review:** an external multi-LLM "Context Library v2 — Engineering Brief"
  (lives in the CL at `projects/bot-hq/ideas.md`, not git-tracked), produced by feeding
  frontier models the CL-research prompt this session generated.

---

## TL;DR

The brief is excellent and largely correct. **Adopt its philosophy, failure-mode catalog,
measurement plan, and the two cheap "Phase 1" fixes wholesale.** But **invert its emphasis**:
lead with codebase legibility + curation (delete/migrate the rot), and treat the
atom/retrieval engine as a thin, **FTS5-first, measurement-gated residual** — defer embeddings
until a measurement proves they're needed. Several of the brief's premises are wrong or much
harder *specifically for bot-hq*; those corrections are below.

---

## 1. Verdict

A genuinely strong brief — coherent, rigorous, and it **independently rediscovered bot-hq's own
stated doctrine** ("ARCHITECTURE.md is the map, the CL is for learnings"; "if grep recovers it
in seconds it doesn't belong here"). When an outside model converges on the house rules from a
cold prompt, that's strong evidence the rules are right. Keep §0 (philosophy), §10 (failure
modes), and §12 (what to measure) almost verbatim.

---

## 2. Adopt wholesale (low / no code)

- **§0 philosophy.** The CL is a fallible notebook *subordinate to* code-as-source-of-truth;
  reads have side effects (a stale atom biases the whole generation), so **a stale atom is worse
  than a missing one.** Put a standing **"reality wins — verify against live code/tests, then
  propose a correction"** line into the cold-start contract.
- **§6 store-this / re-derive-that line.** The single most useful content rule: if a fact is
  cheaply recoverable from current code/tests/git, do **not** store it.
- **§10 failure modes.** Especially **#3** — store *evidence + conditions*, **never a bare
  verdict** (a bare "approach X fails" means X is never re-tested) → make `evidence` mandatory on
  gotcha / failed-approach entries. And **#4** — mandatory `scope` / negative boundaries.
- **§12 measurement.** We already have prior art: the SWE-bench harness (`bench/swebench/`) does
  CL-off runs, and an own-repo retro benchmark was already *scoped* (notes.md, 2026-06-15).

---

## 3. bot-hq-specific corrections (what an outside model couldn't know)

**C1 — The "keystone inversion" is already half-true.** §2 frames "markdown = truth, SQLite =
disposable derived brain, `rebuild` regenerates it" as the big move. **That's already how bot-hq
works**: `cl_rescan` rebuilds `cl_index` from the filesystem; the fs is truth, SQLite is
disposable. The real gap is that the derived layer is *thin* (description + tags) vs. *rich*
(scores, refs, embeddings, versions). It's an enrichment, not an inversion.

**C2 — "Close the escape hatch" (§7) is the weakest part here, and it fights our design.** The
brief wants to stop the agent `Read`-ing 38K of `notes.md` by hiding the path and intercepting
oversized reads. For bot-hq: "keep the path out of the prompt" **directly contradicts** the
deliberate layer-2b design (we *inject* CL paths so index-first works), and "intercept oversized
reads" is a new mechanism because **our Tool Gate is Bash-only** (`src/policy/hooks.rs:799`
returns `None` for any non-Bash tool; `Read` is ungated — verified). Even gating `Read` leaves
`cat`/`grep` via Bash. The durable lever is *make atoms good enough + stop advertising the big
files*, not fencing.

**C3 — Embeddings are the highest cost/risk item here, and probably skippable at first.** No
vector/ONNX infra exists today (verified: no `ort`/`onnx`/`tract`/`candle`/`fastembed`/
`sqlite-vec` dep). A local ONNX encoder is a net-new heavy dependency bundled into a Tauri app
that **already doesn't compile on Windows** and has a fragile release matrix (notes.md). FTS5 is
almost certainly already available (bundled `libsqlite3-sys` 0.30 — **probe with a
`CREATE VIRTUAL TABLE … USING fts5` before relying on it**). **Lexical (FTS5) + scope-match +
kind-specific freshness + pinning ≈ 80% of the retrieval win at ~20% of the cost/risk.**

**C4 — Three "new" pieces are already scaffolded → the big build is cheaper than it looks.**
- `review_queue` ≈ our existing `findings` table + tray + `disposition_finding` flow (the
  EYES-sign-off propose→approve spine). Reuse it.
- The cold-start digest drops straight into `read_system_prompt` **layer 2b** — no need for the
  speculative "write a generated CLAUDE.md into the workdir" trick.
- A/B infra is half-built (swebench CL-off + the scoped own-repo retro benchmark).

---

## 4. Strategic reframe (the most important point)

**§13 — "make the live codebase legible; that beats curated prose memory" — applies *doubly* to
bot-hq, and it quietly undercuts the brief's own centerpiece.** We already keep
ARCHITECTURE/PLAN/PROGRESS **in-repo, versioned with the code**. And by the brief's own §6 rule,
**most of `notes.md` (the ~80KB of dated "Learnings", full of `file:line`, function names, and
commit hashes) is the "re-derive, don't store" category — it's rot, not knowledge.** Atomizing it
faithfully (Phase 2's "each dated bullet → an atom") would just **preserve the rot in a fancier
container.**

So the honest move the brief implies but underweights: **most of `notes.md` should be deleted or
migrated into in-repo docs, and only the thin genuinely-code-invisible residue (the "why", the
evidence-backed traps) becomes structured knowledge.** Curation + in-repo migration is likely the
single biggest win — and it shrinks (or moots) much of the atom engine.

---

## 5. Recommended path (inverted emphasis)

### Phase 1 — surgical fixes, ship now (hours)
- **Fix B (frozen descriptions):** today the rescan detects a changed file by **mtime** (there is
  **no content/`body_hash` stored** — verified) and calls `touch_cl_index`
  (`storage/cl_index.rs:47`), which bumps only the timestamp. Fix = on that change branch, also
  re-run `extract_description` (`bridge/util.rs:79`) and update the description. (Optional
  enhancement: store a content hash so no-op touches don't re-derive — not required for the fix;
  a `body_hash` is otherwise a v2/Phase-3 addition, per the brief's atom schema.) Seam: the rescan
  path in `bridge/cl_facade.rs` + `storage/cl_index.rs`.
- **Fix C (cold-start primer surfaces ephemera):** change `render_cl_primer`
  (`core/session.rs:1000`) to **pin `conventions.md` / `decisions.md` / policy and exclude
  `plans/*`**, then fill the remaining slots by recency — instead of pure `updated_at DESC` top-12.
- **Safety (content discipline, ~0 code):** add the "reality wins" line to the cold-start
  contract; start requiring `evidence` on new gotchas / failed-approaches.

### Phase 2 — legibility + curation (the real ROI)
- **HITL maintenance pass:** delete the re-derivable rot from `notes.md`; migrate the durable
  ~8KB (vision, anti-patterns, commit-hook behavior, gotchas, where-things-live) into in-repo docs
  (ARCHITECTURE/PLAN, or a git-tracked NOTES). Measure tokens-per-task before/after.
- **TTL / exclude handoff docs** (`plans/*`) from the active + primer set.

### Phase 3 — measured retrieval engine (only if a gap remains after Phase 2)
- **FTS5-based ranked `cl.retrieve`** (BM25 + scope-match + kind-specific freshness + pin + MMR/RRF),
  returning **atom bodies inline under a token budget**, with retrieval-time **stale-flagging via
  `code_hash`**. Reuse `findings`/tray for `cl.propose` → review queue (agents propose, humans
  approve — fits our HITL model).
- **Embeddings: later, and only if measurement shows the lexical version misses.** Additive
  migration for any new tables — never edit applied migrations (see notes.md).

---

## 6. What to measure (the gate for Phase 3)

- **Tokens-per-task to acquire context** (headline; target a large drop from "Read 38K notes.md").
  Whole-file-read / escape-hatch rate → near zero.
- **Active CL-token (or atom) count must PLATEAU**, not grow monotonically — the direct test that
  Problem A is dead.
- **CL-on vs CL-off crossover replay** on matched tasks (reuse swebench plumbing): save prompt +
  starting commit + budget, replay both ways, judge by tests + human review.
- **CL-poison eval:** deliberately let a stale entry conflict with current code; measure whether
  the agent blindly obeys or verifies against source. This single test tells you whether the
  "trust memory over reality" failure mode is live.

---

## 7. Risks / open questions

- **FTS5 availability** — probe before relying (likely fine with bundled `libsqlite3-sys`).
- **ONNX cross-platform** vs. bot-hq's Windows / release fragility → defer embeddings.
- **Migration cost** — the full atom model is ~6 new tables on an immutable-migration sqlx setup;
  additive only, don't touch applied migrations.
- **Cloud-sync concurrency** (brief §14) — markdown is cloud-synced but not git-tracked;
  disposable-SQLite saves the index, not the markdown. Unaddressed.
- **Curation is recurring human labor** even with approver-not-author (brief §14); the approval
  gate that prevents poison never goes to zero.
- **Base-model upgrades obsolete "lessons"** (brief §14) — keep the CL small + intent/policy-focused,
  not encyclopedic.

---

## 8. Recommendation in one line

Take the brief's philosophy, failure modes, measurement plan, and Phase-1 fixes wholesale — then
**lead with code legibility and rot-deletion, and build the retrieval engine FTS5-first and only
as far as a measurement justifies.** Don't build the embedding/atom engine on spec.
