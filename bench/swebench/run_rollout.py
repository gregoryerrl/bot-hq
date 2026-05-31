#!/usr/bin/env python3
"""SWE-bench Verified rollout harness for the bot-hq duo (Phase 1 of 2).

Drives bot-hq (Brian/HANDS Opus 4.8 + Rain/EYES DeepSeek-V4-Pro) over N
instances via the external driver MCP and emits predictions.jsonl. Phase 2
(scoring in Docker) is the stock SWE-bench harness — see README.

Requires bot-hq running with its window OPEN (no headless mode; the external
MCP is in-process).

  python run_rollout.py --dry-run            # preflight + dataset + checkout, $0
  python run_rollout.py --n 5                # 5-instance live smoke
"""
from __future__ import annotations

import argparse
import json
import pathlib
import subprocess
import time

import dataset as ds
from bothq_client import BotHqClient, BotHqError
from completion import evaluate, max_id, pick_choice_option
from prompt import format_prompt
from verify import verify_existing_tests

MODEL_NAME = "bot-hq-duo/brian-opus-4.8+rain-deepseek-v4-pro"


def log(msg: str) -> None:
    print(f"[{time.strftime('%H:%M:%S')}] {msg}", flush=True)


def git(repo: pathlib.Path, *args: str, check: bool = True) -> str:
    r = subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True)
    if check and r.returncode != 0:
        raise RuntimeError(f"git {' '.join(args)}: {r.stderr.strip()}")
    return r.stdout


_ARTIFACT_IGNORES = [
    "# bot-hq harness — keep agent-created artifacts out of the extracted patch",
    ".venv*/", "venv/", "env/", "ENV/", ".env/", "__pycache__/", "*.pyc",
    "*.egg-info/", ".eggs/", "build/", "dist/", ".pytest_cache/", ".tox/",
    "node_modules/", ".mypy_cache/", ".hypothesis/",
]


def _seed_exclude(repo_dir: pathlib.Path) -> None:
    """Write artifact patterns to .git/info/exclude — local, untracked, invisible
    to diffs — so `git add -A` never sweeps an agent-created venv/cache into the
    patch (sympy-12419 produced a 39MB diff before this). Idempotent."""
    try:
        (repo_dir / ".git" / "info" / "exclude").write_text("\n".join(_ARTIFACT_IGNORES) + "\n")
    except OSError:
        pass


def ensure_repo(repo: str, cache_root: pathlib.Path) -> pathlib.Path:
    """Full clone (cached, reused across instances of the same repo)."""
    dest = cache_root / repo.replace("/", "__")
    if (dest / ".git").exists():
        _seed_exclude(dest)
        return dest
    cache_root.mkdir(parents=True, exist_ok=True)
    url = f"https://github.com/{repo}.git"
    log(f"  cloning {url} (first time, may be slow)…")
    r = subprocess.run(["git", "clone", "--quiet", url, str(dest)], capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(f"clone {url}: {r.stderr.strip()}")
    _seed_exclude(dest)
    return dest


def checkout_base(repo_dir: pathlib.Path, base_commit: str) -> None:
    """Pristine working tree at base_commit (HEAD == base_commit afterwards)."""
    git(repo_dir, "reset", "--hard", "--quiet", check=False)
    git(repo_dir, "clean", "-fdxq", check=False)
    present = subprocess.run(
        ["git", "-C", str(repo_dir), "cat-file", "-e", f"{base_commit}^{{commit}}"]
    ).returncode == 0
    if not present:
        log(f"  fetching origin for {base_commit[:10]}…")
        git(repo_dir, "fetch", "--quiet", "origin", check=False)
    git(repo_dir, "checkout", "--quiet", "--force", base_commit)


def extract_patch(repo_dir: pathlib.Path) -> str:
    """The model_patch: everything the agent changed vs base_commit (HEAD)."""
    git(repo_dir, "add", "-A")
    return git(repo_dir, "diff", "--cached")


def preflight(client: BotHqClient, args: argparse.Namespace) -> None:
    log("preflight: get_status")
    st = client.get_status()
    log(f"  bot-hq v{st.get('version')} | ext-mcp {st.get('external_mcp_addr')} | active duos {st.get('active_duo_sessions')}")
    cfgs = {c["agent_name"]: c for c in client.get_agent_configs()}
    brian, rain = cfgs.get("brian", {}), cfgs.get("rain", {})
    log(f"  brian model = {brian.get('model_name')!r}")
    log(f"  rain  model = {rain.get('model_name')!r} | base_url = {rain.get('base_url')!r}")
    bad = []
    if "opus" not in (brian.get("model_name") or "").lower():
        bad.append(f"brian not opus ({brian.get('model_name')})")
    if "deepseek" not in (rain.get("model_name") or "").lower():
        bad.append(f"rain not deepseek ({rain.get('model_name')})")
    if bad:
        if args.skip_config_check:
            log(f"  WARN config mismatch ignored: {'; '.join(bad)}")
        else:
            raise SystemExit(f"preflight FAILED: {'; '.join(bad)}. Fix in Settings or pass --skip-config-check.")
    log("  note: llm_proxy uses an ephemeral port (not pollable) — Rain-liveness is verified per instance.")


def _poll_until_done(client: BotHqClient, sid: str, args: argparse.Namespace):
    """Drive the duo until it signals completion (one solve attempt).
    Returns (reason, rain_spoke, brian_spoke, choices_resolved)."""
    start = time.monotonic()
    since, last_msg_t = 0, start
    rain_spoke = brian_spoke = False
    resolved = 0
    errors = 0  # consecutive MCP transport errors
    reason = "loop-exit"
    while True:
        # Top guard: terminate on the hard cap regardless of which call errors
        # (an erroring snapshot below would otherwise skip the evaluate() check).
        if time.monotonic() - start >= args.max_seconds:
            reason = f"wall-clock cap {args.max_seconds:.0f}s reached"
            break
        if errors >= 5:
            reason = "aborted: 5 consecutive MCP errors (bot-hq down?)"
            break
        try:
            msgs = client.wait_for_change(sid, since_id=since, timeout_ms=20000)
            errors = 0
        except BotHqError as e:
            log(f"  wait_for_change: {e} (retrying)")
            msgs = []
            errors += 1
            time.sleep(2)
        now = time.monotonic()
        if msgs:
            since = max_id(msgs, since)
            last_msg_t = now
            for m in msgs:
                if m.get("author") == "rain":
                    rain_spoke = True
                elif m.get("author") == "brian":
                    brian_spoke = True
        try:
            snap = client.get_session_snapshot(sid, msg_limit=1)
            errors = 0
        except BotHqError as e:
            log(f"  snapshot: {e} (retrying)")
            errors += 1
            time.sleep(2)
            continue
        pend = snap.get("pending_choices") or []
        for c in pend:
            picked = pick_choice_option(c.get("options") or [])
            try:
                client.resolve_choice(c["choice_id"], picked)
                resolved += 1
                log(f"  auto-resolved choice -> {picked!r}")
            except BotHqError as e:
                log(f"  resolve_choice failed: {e}")
        d = evaluate(
            awaiting=bool(snap.get("awaiting")),
            pending_choice_count=len(pend),
            seconds_since_last_msg=now - last_msg_t,
            elapsed=now - start,
            silence_timeout=args.silence_timeout,
            wall_clock_cap=args.max_seconds,
        )
        if d.done:
            reason = d.reason
            break
    return reason, rain_spoke, brian_spoke, resolved


def _verify_feedback(failing: list[str], details: str) -> str:
    more = f"\n  …and {len(failing) - 12} more" if len(failing) > 12 else ""
    body = details or "\n".join(f"  - {t}" for t in failing[:12])
    return (f"Your change BROKE {len(failing)} existing tests that passed before "
            "your edit. Here are the failures WITH their errors:\n\n"
            f"{body}{more}\n\n"
            "A correct fix must NOT break existing behaviour. Read the errors above, "
            "identify what your patch changed that caused them, and revise so these "
            "tests pass again while still resolving the issue. Then call "
            "mark_awaiting_user('SWEBENCH_DONE') again when done.")


def run_instance(client: BotHqClient, inst: dict, repo_dir: pathlib.Path, args: argparse.Namespace):
    iid = inst["instance_id"]
    sid = client.create_session(title=f"swebench:{iid}", working_repo_path=repo_dir.resolve())
    log(f"  session {sid}")
    client.send_message(sid, format_prompt(inst, str(repo_dir.resolve())))
    t0 = time.monotonic()
    rain_spoke = brian_spoke = False
    resolved = rounds = 0
    verify_summary = None
    reason = "loop-exit"
    while True:
        reason, rs, bs, res = _poll_until_done(client, sid, args)
        rain_spoke, brian_spoke, resolved = rain_spoke or rs, brian_spoke or bs, resolved + res
        diff = extract_patch(repo_dir)
        if not args.verify or rounds >= args.verify_rounds or not diff.strip():
            break
        log(f"  verify round {rounds + 1}: running existing tests against the patch…")
        try:
            failing, summary, ok, details = verify_existing_tests(inst, diff)
        except Exception as e:  # noqa: BLE001
            log(f"  verify error: {e} (skipping feedback)")
            break
        verify_summary = summary
        if not ok:
            log(f"  verify inconclusive ({summary}) — not bouncing")
            break
        if not failing:
            log(f"  verify CLEAN ({summary}) — no regressions")
            break
        rounds += 1
        log(f"  verify: {len(failing)} existing-test regressions ({summary}) -> bounce back to duo")
        try:
            client.send_message(sid, _verify_feedback(failing, details))
        except BotHqError as e:
            log(f"  send_message (verify feedback) failed: {e}")
            break
        # loop: re-poll for the duo's revised SWEBENCH_DONE
    elapsed = time.monotonic() - t0
    patch = extract_patch(repo_dir)
    try:
        client.close_session(sid)
    except BotHqError as e:
        log(f"  close_session: {e}")
    vinfo = (f" | verify_rounds={rounds}" + (f" ({verify_summary})" if verify_summary else "")) if args.verify else ""
    log(f"  END {iid}: {reason} | patch {len(patch)}B | rain_spoke={rain_spoke}{vinfo} | {elapsed:.0f}s")
    if not rain_spoke:
        log(f"  WARN Rain never spoke for {iid} — DeepSeek/llm_proxy may be down (EYES half dead).")
    pred = {"instance_id": iid, "model_name_or_path": MODEL_NAME, "model_patch": patch}
    meta = {"instance_id": iid, "reason": reason, "elapsed_s": round(elapsed, 1),
            "patch_bytes": len(patch), "rain_spoke": rain_spoke, "brian_spoke": brian_spoke,
            "choices_resolved": resolved, "verify_rounds": rounds, "verify_summary": verify_summary}
    return pred, meta


def main() -> None:
    ap = argparse.ArgumentParser(description="SWE-bench Verified rollout harness for the bot-hq duo")
    ap.add_argument("--n", type=int, default=5)
    ap.add_argument("--offset", type=int, default=0, help="dataset offset; deterministic first-N from here")
    ap.add_argument("--instance-ids", default=None,
                    help="comma-separated instance_ids to run; overrides --n/--offset")
    ap.add_argument("--out", default="runs/smoke/predictions.jsonl")
    ap.add_argument("--url", default="http://127.0.0.1:7892/mcp")
    ap.add_argument("--token-file", default=None, help="path to mcp-token (default <data_dir>/mcp-token)")
    ap.add_argument("--cache", default="runs/repo_cache")
    ap.add_argument("--max-seconds", type=float, default=600.0, help="hard cap per instance")
    ap.add_argument("--silence-timeout", type=float, default=120.0, help="end instance after this much silence")
    ap.add_argument("--skip-config-check", action="store_true")
    ap.add_argument("--dry-run", action="store_true", help="preflight + dataset + checkout only; NO sessions, $0")
    ap.add_argument("--verify", action="store_true",
                    help="test-feedback loop: after SWEBENCH_DONE, run existing tests against the patch "
                         "(prebuilt container, no test_patch) and bounce regressions back to revise")
    ap.add_argument("--verify-rounds", type=int, default=2, help="max verify-and-revise rounds (with --verify)")
    args = ap.parse_args()

    out = pathlib.Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    cache = pathlib.Path(args.cache)
    client = BotHqClient(url=args.url, token_path=args.token_file)

    preflight(client, args)

    if args.instance_ids:
        ids = [s.strip() for s in args.instance_ids.split(",") if s.strip()]
        log(f"loading {len(ids)} instances by id from SWE-bench_Verified…")
        instances, truncated = ds.load_by_ids(ids)
    else:
        log(f"loading {args.n} instances (offset {args.offset}) from SWE-bench_Verified…")
        instances, truncated = ds.load_instances(args.n, offset=args.offset)
    extra = f"; WARN {truncated} truncated by rows-API (use datasets lib for full fidelity)" if truncated else ""
    log(f"  got {len(instances)} instances{extra}")

    predictions, metas = [], []
    meta_path = out.with_name("meta.jsonl")

    def flush():
        with out.open("w") as f:
            for p in predictions:
                f.write(json.dumps(p) + "\n")
        with meta_path.open("w") as f:
            for m in metas:
                f.write(json.dumps(m) + "\n")

    for i, inst in enumerate(instances, 1):
        iid = inst.get("instance_id", f"<row {i}>")
        log(f"[{i}/{len(instances)}] {iid} ({inst.get('repo')})")
        pred = {"instance_id": iid, "model_name_or_path": MODEL_NAME, "model_patch": ""}
        meta = {"instance_id": iid, "reason": "?", "patch_bytes": 0}
        try:
            repo_dir = ensure_repo(inst["repo"], cache)
            checkout_base(repo_dir, inst["base_commit"])
            if args.dry_run:
                log(f"  DRY: repo @ {inst['base_commit'][:10]} | issue {len(inst.get('problem_statement') or '')} chars")
                continue
            pred, meta = run_instance(client, inst, repo_dir, args)
        except Exception as e:  # noqa: BLE001 — record + keep going
            log(f"  INSTANCE FAILED: {e}")
            meta = {"instance_id": iid, "reason": f"error: {e}", "patch_bytes": 0}
        predictions.append(pred)
        metas.append(meta)
        flush()  # incremental write — survive a mid-run abort under tight budget

    if args.dry_run:
        log("DRY RUN complete — no sessions spawned, no cost.")
        return

    nonempty = sum(1 for p in predictions if p["model_patch"].strip())
    rain_live = sum(1 for m in metas if m.get("rain_spoke"))
    log(f"wrote {len(predictions)} predictions -> {out}")
    log(f"wrote diagnostics    -> {meta_path}")
    log(f"SUMMARY: {nonempty}/{len(predictions)} non-empty patches | Rain spoke in {rain_live}/{len(metas)} instances")
    log("Next (Phase 2): score with the stock SWE-bench harness — see README.")


if __name__ == "__main__":
    main()
