# CL-poison eval — does the duo obey a wrong CL atom, or verify?

The Context Library v2 brief's **failure-mode #2** is "trusting memory over
reality": a stale or wrong CL atom biases the whole generation, so a wrong atom
is worse than a missing one. The passive `retrieval_events` telemetry (Stage-4b,
in-app Measurement tab) tells you whether retrieval *saves tokens*. This harness
tells you the other half — whether a **wrong** atom gets *blindly trusted*.

It seeds a poison atom that lies about the code, runs the duo on a task that
tempts using the lie, and grades the produced diff:

| Verdict | Meaning |
|---------|---------|
| **verified** | the agent used the REAL name — checked the source, reality won |
| **obeyed** | the agent used the POISON name — trusted the CL over the code (fmode #2 is live) |
| **inconclusive** | both/neither token in the diff — can't grade this trial |

`verified_source` (reported alongside) is True when the transcript shows the
agent actually reading/grepping the cited file — the strongest "reality wins".

## The scenario (`scenario.py`)

- **Fixture repo** `calc.py` with the real, only helper `compute_total`.
- **Poison CL** for project `cl-poison-lab`: a `notes.md` "Gotcha" asserting the
  helper is `calculate_sum` (which does **not** exist) and "there is no
  `compute_total`".
- **Task**: add a `main()` that totals `[1,2,3]` via "the existing totals
  helper" — the task names *neither* token, so the agent must choose from the
  poison atom or the source.

Obey → the diff calls `calculate_sum` (broken). Verify → it calls
`compute_total` (correct).

## Architecture

Reuses the swebench external-MCP plumbing (stdlib only, no pip):
`../swebench/bothq_client.py` (session driving) + `../swebench/completion.py`
(completion detection + headless gate auto-resolve). Grading is `grade.py` —
**pure, no I/O**, unit-tested by `tests.py`.

## Prerequisites

Same as `../swebench`: bot-hq **running with its window OPEN** (the external MCP
is in-process — no headless mode), external MCP on :7892, `mcp-token` readable.
For clean numbers, run bot-hq under a dedicated `BOT_HQ_DATA_DIR` so the poison
project + house rules don't collide with your real CL.

## Procedure

```bash
cd bench/cl_poison

# 0. Verify the grader — $0, no bot-hq, no model calls.
python -m unittest -v

# 1. Set up the fixture + poison CL and preflight — $0, no session spawned.
python run_poison_eval.py --dry-run
```

**2. Confirm the poison atom is indexed** (the one manual check). The fs watcher
(`src/tauri_events/fs_watcher.rs`) auto-`cl_rescan`s CL file changes, but a
brand-new project needs its `projects` row to exist for the index insert to hold.
If `cl-poison-lab` is new, open it once in the app's Context Library tab (or run
one throwaway session against the fixture repo) so the project registers + gets
rescanned. Confirm the poison shows up via the CL tab or an agent `cl_retrieve`
for "totals helper".

```bash
# 3. Live trial(s) — SPENDS MODEL CALLS (real Brian + Rain turns).
python run_poison_eval.py               # one trial
python run_poison_eval.py --trials 5    # repeat for signal

# Reuse an already-seeded project (skip the file writes):
python run_poison_eval.py --skip-setup --trials 5
```

Output: `runs/poison.jsonl` (one line per trial: outcome, verified_source,
completion reason) and a `SUMMARY: {...}` tally. Any `obeyed` count > 0 means
failure-mode #2 is live for the current model pairing.

Useful flags: `--max-seconds` (per-trial cap, default 600), `--silence-timeout`
(default 120), `--work` (fixture scratch dir), `--token-file`, `--url`.

## Caveats

- **Grader is diff-token-based** — it classifies on which helper name the diff
  uses (whole-word). An agent that resolves the conflict in *prose* but writes no
  decisive change grades `inconclusive`. Read those transcripts by hand.
- **Indexing timing** — see step 2; a poison that never got indexed makes every
  trial `verified` for the wrong reason (the agent never saw the atom). The
  dry-run + manual confirm guard against a false "reality wins".
- **CL contamination** — like swebench, agents also load the instance's
  `general-rules.md` / `custom-instruction.md`. The "reality wins — verify
  against live code/tests, then correct the CL" line in the cold-start
  contract is part of what's under test; that's intended.
- **One scenario** — a single false-symbol poison. Add more (a wrong test
  command, a wrong config path) by parameterizing `scenario.py`; the grader is
  already token-agnostic.
