# SWE-bench Verified harness for the bot-hq duo

Evaluates the **bot-hq duo** — Brian (HANDS, Opus 4.8) + Rain (EYES,
DeepSeek-V4-Pro) — on [SWE-bench Verified](https://www.swebench.com/) (500
human-validated GitHub-issue→patch tasks).

SWE-bench is two phases. This harness **replaces Phase 1** (produce a patch per
instance) by driving bot-hq through its external MCP. **Phase 2** (score the
patches in Docker) is the stock upstream harness — unchanged.

```
Phase 1  run_rollout.py ── external MCP :7892 ──▶ bot-hq spawns Brian+Rain per
  (this dir)                                       instance, they edit the repo
                                                   │
                                                   ▼  git diff
                                          predictions.jsonl
                                                   │
Phase 2  python -m swebench.harness.run_evaluation │  (Docker per instance)
  (upstream)                                       ▼
                                          resolved / unresolved report
```

No bot-hq code changes — pure external-driver client. The rollout harness is
**stdlib-only** (no pip install): it talks to bot-hq over `urllib` and pulls the
dataset from the HF datasets-server REST API.

## Prerequisites

- **bot-hq running with its window OPEN.** There is no headless mode — the
  external MCP server lives in-process, so the GUI binary must be up. Fine
  locally; it can't run on a displayless CI box.
- External MCP enabled (default; it's off only if `BOT_HQ_EXTERNAL_MCP_DISABLED=1`).
  Confirm port 7892 is listening.
- Agent configs set to the target pairing (Settings tab): `brian` →
  `claude-opus-4-8`; `rain` → `deepseek-v4-pro` (DeepSeek gateway via the
  llm_proxy). The harness preflight verifies this and aborts on mismatch.
- `git`. Docker only for Phase 2.

## Run

```bash
cd bench/swebench

# 0. Validate everything WITHOUT spending money: preflight + dataset + repo
#    checkout, but no sessions are spawned.
python run_rollout.py --dry-run --n 5

# 1. Live 5-instance smoke (spends a little — real Opus + DeepSeek turns).
python run_rollout.py --n 5

# Full run later (parallelism + worktree isolation still TODO — see Caveats).
python run_rollout.py --n 500 --out runs/full/predictions.jsonl
```

Outputs: `runs/<name>/predictions.jsonl` (the SWE-bench prediction file:
`{instance_id, model_name_or_path, model_patch}` per line) and a sibling
`meta.jsonl` with per-instance diagnostics (completion reason, elapsed,
patch size, whether Rain actually spoke, choices auto-resolved).

Useful flags: `--max-seconds` (hard cap/instance, default 600), `--silence-timeout`
(default 120), `--offset` (deterministic window into the dataset),
`--skip-config-check`, `--token-file`, `--url`.

## Phase 2 — scoring (needs Docker)

First scored runs of the duo: **27/39 resolved (69%)** across all 12 SWE-bench
Verified repos, 0 scoring errors (artifacts in `runs/combined/`). Per-repo
highlights: scikit-learn / sphinx / sympy 3/3, xarray 2/2, flask 1/1; weakest
django / pylint 1/3. The initial 5-instance astropy smoke was 4/5.

```bash
pip install -r requirements-full.txt   # datasets + swebench

# Apple Silicon: PULL prebuilt images (default namespace) + emulate x86.
DOCKER_DEFAULT_PLATFORM=linux/amd64 python -m swebench.harness.run_evaluation \
  --dataset_name princeton-nlp/SWE-bench_Verified \
  --predictions_path runs/smoke/predictions.jsonl \
  --run_id bothq-duo-smoke \
  --namespace swebench \
  --max_workers 1
```

**Hard-won lessons — SWE-bench's image recipes have bit-rotted:**
- Use the **default `--namespace swebench`** so the harness PULLS prebuilt env
  images (where `setup_env.sh` already ran with a compatible pip). Do NOT pass
  `--namespace none` — that forces a local rebuild, which re-runs the stale
  `setup_env.sh` and dies with `TypeError: dataclass() got an unexpected keyword
  argument 'slots'` (pip 24+ calling a 3.10-only API under the testbed's conda
  Python 3.9). This is the single most important flag.
- On Apple Silicon set `DOCKER_DEFAULT_PLATFORM=linux/amd64` to run the x86
  prebuilt images under emulation (~5-6 min/instance). `--max_workers 1` avoids
  OOM from parallel emulated builds.
- `--modal true` REBUILDS images on Modal → hits the exact same rot. Don't use
  it for these older instances.
- `sb-cli` (hosted) is the fallback if a prebuilt image is missing for an instance.

## How completion is detected

bot-hq emits no explicit "agent done" event, so the rollout loop infers it:

- **Primary:** the prompt instructs Brian to call
  `mark_awaiting_user("SWEBENCH_DONE")` when the fix is reviewed. That flips the
  session's `awaiting` flag. Completion = `awaiting == true` **and no pending
  choice** — because `request_approval` *also* sets `awaiting` while parking a
  choice, so a pending choice means "gate to auto-resolve", not "done".
- **Gate auto-resolve:** every parked `ask_user_choice` / `request_approval`
  is answered via `resolve_choice` (prefers a proceed/continue/approve option)
  so a headless run never deadlocks on a human gate.
- **Backstops:** message silence ≥ `--silence-timeout`, hard `--max-seconds` cap.

## Caveats

- **CL contamination:** agents load the running instance's `general-rules.md` +
  `custom-instructions.md`. Against a prod data-dir that's bot-hq house rules
  (CL-first, commit rules) — noise that measures "Brian the maintainer", not a
  clean SWE agent. For real numbers, run bot-hq under a dedicated
  `BOT_HQ_DATA_DIR=~/.bot-hq-swebench/` with neutral instructions +
  `general-policy.yaml { push_gate.mode: auto }`. The hardcoded HANDS/EYES role
  prompts stay — that's the mechanism under test.
- **rows-API truncation:** the stdlib dataset loader can clip very long problem
  statements (it warns when it does). Use the `datasets` lib for full fidelity.
- **Sequential only:** instances run one at a time, reusing one clone per repo.
  Parallelism for the full 500 needs per-instance `git worktree` isolation — TODO.
- **Test-file changes:** the raw `git diff` is used as-is. If an agent edits test
  files, they ride along in the patch; stripping them to match the evaluator's
  own test_patch is a TODO.
