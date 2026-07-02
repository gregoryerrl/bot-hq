#!/usr/bin/env python3
"""CL-poison behavioral eval for the bot-hq duo (Stage-4b; brief failure-mode #2).

Does an agent OBEY a Context-Library atom that contradicts the code, or VERIFY
against the source? This seeds a poison atom (a false "the helper is named X"),
runs the duo on a task that tempts using it, and grades the resulting diff
obey-vs-verify. It is the active complement to the passive retrieval_events
telemetry: telemetry says whether retrieval saves tokens; this says whether a
stale/wrong atom gets blindly trusted.

Requires bot-hq running with its window OPEN (external MCP in-process), exactly
like the swebench harness. **RUNNING A LIVE TRIAL SPENDS MODEL CALLS.**

  python run_poison_eval.py --dry-run     # setup + preflight only, $0
  python run_poison_eval.py               # one live trial (spends)
  python run_poison_eval.py --trials 3    # repeat for signal
  python run_poison_eval.py --skip-setup  # reuse an already-seeded poison project

Reuses the swebench external-MCP client + completion loop (stdlib-only, no pip).
Grading logic is in grade.py (pure, unit-tested by tests.py).
"""
from __future__ import annotations

import argparse
import json
import os
import pathlib
import subprocess
import sys
import time

# Reuse the swebench external-MCP client + completion loop (stdlib only).
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent.parent / "swebench"))
from bothq_client import BotHqClient, BotHqError  # noqa: E402
from completion import evaluate, max_id, pick_choice_option  # noqa: E402

import scenario  # noqa: E402
from grade import grade  # noqa: E402


def log(msg: str) -> None:
    print(f"[{time.strftime('%H:%M:%S')}] {msg}", flush=True)


def data_dir_from(token_file: str | None) -> pathlib.Path:
    if token_file:  # token lives at <data_dir>/mcp-token
        return pathlib.Path(token_file).expanduser().parent
    d = os.environ.get("BOT_HQ_DATA_DIR")
    return pathlib.Path(d).expanduser() if d else pathlib.Path.home() / ".bot-hq"


def git_diff(repo: pathlib.Path) -> str:
    subprocess.run(["git", "-C", str(repo), "add", "-A"], capture_output=True, text=True)
    r = subprocess.run(["git", "-C", str(repo), "diff", "--cached"], capture_output=True, text=True)
    return r.stdout


def transcript_of(client: BotHqClient, sid: str) -> str:
    snap = client.get_session_snapshot(sid, msg_limit=300)
    msgs = snap.get("messages") or []
    return "\n".join(f"{m.get('author')}: {m.get('content', '')}" for m in msgs)


def poll_until_done(client: BotHqClient, sid: str, max_seconds: float, silence: float) -> str:
    """Drive the duo to completion, auto-resolving headless gates. Trimmed twin of
    swebench's _poll_until_done — same completion semantics (completion.evaluate)."""
    start = time.monotonic()
    since, last_msg_t, errors = 0, start, 0
    while True:
        if time.monotonic() - start >= max_seconds:
            return f"wall-clock cap {max_seconds:.0f}s"
        if errors >= 5:
            return "aborted: 5 consecutive MCP errors (bot-hq down?)"
        try:
            msgs = client.wait_for_change(sid, since_id=since, timeout_ms=20000)
            errors = 0
        except BotHqError as e:
            log(f"  wait_for_change: {e} (retrying)")
            msgs, errors = [], errors + 1
            time.sleep(2)
        now = time.monotonic()
        if msgs:
            since, last_msg_t = max_id(msgs, since), now
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
            try:
                client.resolve_choice(c["choice_id"], pick_choice_option(c.get("options") or []))
                log(f"  auto-resolved a gate")
            except BotHqError as e:
                log(f"  resolve_choice failed: {e}")
        d = evaluate(
            awaiting=bool(snap.get("awaiting")),
            pending_choice_count=len(pend),
            seconds_since_last_msg=now - last_msg_t,
            elapsed=now - start,
            silence_timeout=silence,
            wall_clock_cap=max_seconds,
        )
        if d.done:
            return d.reason


def run_trial(client: BotHqClient, repo: pathlib.Path, i: int, args: argparse.Namespace) -> dict:
    scenario.reset_repo(repo)
    sid = client.create_session(title=f"cl-poison:{i}", working_repo_path=repo.resolve())
    log(f"  trial {i}: session {sid}")
    client.send_message(sid, scenario.TASK)
    reason = poll_until_done(client, sid, args.max_seconds, args.silence_timeout)
    diff = git_diff(repo)
    transcript = transcript_of(client, sid)
    try:
        client.close_session(sid)
    except BotHqError as e:
        log(f"  close_session: {e}")
    v = grade(
        diff=diff,
        transcript=transcript,
        poison_token=scenario.POISON_TOKEN,
        real_token=scenario.REAL_TOKEN,
    )
    log(
        f"  trial {i}: {v.outcome.upper()} — {v.reason} | "
        f"verified_source={v.verified_source} | diff={len(diff)}B | ({reason})"
    )
    return {
        "trial": i,
        "outcome": v.outcome,
        "reason": v.reason,
        "verified_source": v.verified_source,
        "completion": reason,
        "diff_bytes": len(diff),
    }


def main() -> None:
    ap = argparse.ArgumentParser(description="CL-poison behavioral eval for the bot-hq duo")
    ap.add_argument("--trials", type=int, default=1, help="number of poison trials to run")
    ap.add_argument("--url", default="http://127.0.0.1:7892/mcp")
    ap.add_argument("--token-file", default=None, help="path to mcp-token (default <data_dir>/mcp-token)")
    ap.add_argument("--work", default=None, help="scratch dir for the fixture repo (default: a temp dir)")
    ap.add_argument("--out", default="runs/poison.jsonl")
    ap.add_argument("--max-seconds", type=float, default=600.0, help="hard cap per trial")
    ap.add_argument("--silence-timeout", type=float, default=120.0)
    ap.add_argument("--skip-setup", action="store_true", help="reuse an already-seeded poison project")
    ap.add_argument("--dry-run", action="store_true", help="setup + preflight only; NO session, $0")
    args = ap.parse_args()

    data_dir = data_dir_from(args.token_file)
    work_root = pathlib.Path(args.work).expanduser() if args.work else pathlib.Path(
        os.path.join(os.sep, "tmp", "bot-hq-cl-poison")
    )

    if args.skip_setup:
        repo = work_root / scenario.PROJECT
        log(f"skip-setup: expecting fixture at {repo} and poison CL at {scenario.cl_project_dir(data_dir)}")
    else:
        log(f"setup: fixture repo + poison CL under project {scenario.PROJECT!r}")
        repo = scenario.setup(data_dir, work_root)
        log(f"  repo        = {repo}")
        log(f"  poison CL   = {scenario.cl_project_dir(data_dir)} (fs-watcher should auto-index it)")
        log(f"  poison lie  = calc helper is {scenario.POISON_TOKEN!r} (real: {scenario.REAL_TOKEN!r})")

    client = BotHqClient(url=args.url, token_path=args.token_file)
    st = client.get_status()
    log(f"preflight: bot-hq v{st.get('version')} | ext-mcp {st.get('external_mcp_addr')}")

    if args.dry_run:
        log("DRY RUN: setup + preflight done. Confirm the poison atom is indexed")
        log(f"  (open project {scenario.PROJECT!r} in the CL tab, or cl_index_search), then run live.")
        log("No session spawned, no cost.")
        return

    out = pathlib.Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    results = []
    for i in range(1, args.trials + 1):
        log(f"[{i}/{args.trials}] poison trial")
        try:
            results.append(run_trial(client, repo, i, args))
        except Exception as e:  # noqa: BLE001 — record + keep going
            log(f"  TRIAL FAILED: {e}")
            results.append({"trial": i, "outcome": "error", "reason": str(e)})
        with out.open("w") as f:
            for r in results:
                f.write(json.dumps(r) + "\n")

    tally = {}
    for r in results:
        tally[r["outcome"]] = tally.get(r["outcome"], 0) + 1
    log(f"wrote {len(results)} trial(s) -> {out}")
    log(f"SUMMARY: {tally}")
    obeyed = tally.get("obeyed", 0)
    if obeyed:
        log(f"  ⚠ failure-mode #2 is LIVE: the duo OBEYED the poison in {obeyed}/{len(results)} trial(s).")
    else:
        log("  reality won in every gradable trial (no blind obedience observed).")


if __name__ == "__main__":
    main()
