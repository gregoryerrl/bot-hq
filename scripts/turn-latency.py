#!/usr/bin/env python3
"""What a session FELT like, measured the same way every time.

    scripts/turn-latency.py                     # every session in the live db
    scripts/turn-latency.py s-a4e9a1            # one session (prefix matches)
    scripts/turn-latency.py --db <path> ...     # e.g. a pre-reset legacy db

Exists because the answer changed with the question on 2026-08-13. Asked "are
turns slower on rc3", three different measurements gave 8.2s, 11.1s and 30.1s —
all correct, all measuring something different, and the first one was reported
as if it answered the user's question. It did not: it counted ANY change of
author as a handoff, so a participant writing out of turn (the D24 wedge) looked
like a fast handoff and pulled the median down.

So the metrics are named, and the one that matches what a person actually
watches is first.

  REPLY   you type -> the first participant row appears. The only number a user
          experiences directly. Comparable across builds: it needs nothing but
          `messages`, so a pre-rc3 database answers it too.

  GAP     one participant's last row -> the next participant's first row, at a
          real ring turn boundary, with every gap the user spoke in removed.
          rc3 only — it reads `participant_deliveries` for the boundaries, and
          pre-rc3 has no ring to have boundaries.

  SPLIT   GAP, decomposed: how much was the turn ENDING (last row -> the ring
          handing over) versus the next participant STARTING (handover -> its
          first row). The first half is where a discarded completion shows up;
          the second is prefill.

  PACE    gap between consecutive rows by the same participant — the model
          working, with no handoff and no user in it. The control: if this moves
          between builds, the harness is not what changed.
"""

import argparse
import datetime
import os
import sqlite3
import sys

DEFAULT_DB = os.path.expanduser("~/.bot-hq/.local/bot-hq.db")


def ts(s):
    """RFC3339-Z or sqlite's zone-less shape -> naive datetime, or None."""
    if not s:
        return None
    s = s.replace("Z", "").replace("T", " ")
    if "." not in s:
        s += ".0"
    try:
        return datetime.datetime.strptime(s, "%Y-%m-%d %H:%M:%S.%f")
    except ValueError:
        return None


def pct(sorted_vals, p):
    if not sorted_vals:
        return None
    return sorted_vals[min(len(sorted_vals) - 1, int(len(sorted_vals) * p))]


def summarise(name, vals, unit="s"):
    v = sorted(x for x in vals if x is not None)
    if not v:
        return f"  {name:<7} —"
    return (
        f"  {name:<7} n={len(v):<4} median {pct(v,0.5):>7.1f}{unit}"
        f"   p75 {pct(v,0.75):>7.1f}{unit}   p90 {pct(v,0.9):>7.1f}{unit}"
        f"   max {v[-1]:>7.1f}{unit}"
    )


def rows_of(conn, sid):
    out = []
    for created, origin, kind, author, pid in conn.execute(
        "SELECT created_at, origin, kind, author, participant_id FROM messages "
        "WHERE session_id = ? ORDER BY id",
        (sid,),
    ):
        at = ts(created)
        if at:
            out.append((at, origin, kind, author, pid))
    return out


def reply_latency(rows):
    """You type -> the first participant row. Capped at 30min: past that the
    session was waiting on a human who walked away, which is not latency."""
    out = []
    for i, (at, origin, kind, _, _) in enumerate(rows):
        if origin != "user" or kind != "text":
            continue
        nxt = next((r[0] for r in rows[i + 1 :] if r[1] == "participant"), None)
        if nxt:
            g = (nxt - at).total_seconds()
            if 0 <= g < 1800:
                out.append(g)
    return out


def pace(rows):
    """Consecutive rows by the SAME participant."""
    out = []
    for i in range(1, len(rows)):
        a, b = rows[i - 1], rows[i]
        if a[1] != "participant" or b[1] != "participant":
            continue
        if a[4] != b[4]:
            continue
        g = (b[0] - a[0]).total_seconds()
        if 0 <= g < 600:
            out.append(g)
    return out


def ring_gaps(conn, sid, rows):
    """GAP and SPLIT, off the ring's own turn boundaries.

    A turn boundary is a delivery BATCH: `participant_deliveries` rows sharing a
    `delivered_at` are one handover. Empty on a pre-rc3 database, which is the
    honest answer there rather than a number derived some other way.
    """
    turns = [
        (ts(a), p)
        for a, p in conn.execute(
            "SELECT d.delivered_at, d.participant_id FROM participant_deliveries d "
            "JOIN messages m ON m.id = d.message_id WHERE m.session_id = ? "
            "GROUP BY d.delivered_at, d.participant_id ORDER BY d.delivered_at",
            (sid,),
        )
    ]
    turns = [t for t in turns if t[0]]
    users = [r[0] for r in rows if r[1] == "user"]
    gap, ending, starting = [], [], []
    for i in range(len(turns) - 1):
        start, pid = turns[i]
        nxt_start, nxt_pid = turns[i + 1]
        mine = [r[0] for r in rows if r[4] == pid and start <= r[0] < nxt_start]
        theirs = [r[0] for r in rows if r[4] == nxt_pid and r[0] >= nxt_start]
        if not mine or not theirs:
            continue
        last, first = mine[-1], theirs[0]
        # The user spoke in this gap: that is a human thinking, not the harness.
        if any(last < u < first for u in users):
            continue
        total = (first - last).total_seconds()
        if total < 0:
            continue
        gap.append(total)
        ending.append((nxt_start - last).total_seconds())
        starting.append((first - nxt_start).total_seconds())
    return gap, ending, starting


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("session", nargs="?", help="session id or prefix")
    ap.add_argument("--db", default=DEFAULT_DB)
    ap.add_argument("--limit", type=int, default=12)
    args = ap.parse_args()

    if not os.path.exists(args.db):
        sys.exit(f"no database at {args.db}")
    conn = sqlite3.connect(f"file:{args.db}?mode=ro", uri=True)

    q = "SELECT id, title, created_at FROM sessions"
    params = ()
    if args.session:
        q += " WHERE id LIKE ?"
        params = (args.session + "%",)
    q += " ORDER BY created_at DESC LIMIT ?"
    params += (args.limit,)

    everything = {"REPLY": [], "GAP": [], "PACE": []}
    for sid, title, created in conn.execute(q, params):
        rows = rows_of(conn, sid)
        if len(rows) < 5:
            continue
        gap, ending, starting = ring_gaps(conn, sid, rows)
        reply, p = reply_latency(rows), pace(rows)
        everything["REPLY"] += reply
        everything["GAP"] += gap
        everything["PACE"] += p
        print(f"\n{sid}  {(title or '')[:38]}  {created[:19]}")
        print(summarise("REPLY", reply))
        print(summarise("GAP", gap))
        if gap:
            print(summarise("  ending", ending))
            print(summarise("  start", starting))
        print(summarise("PACE", p))

    print("\n" + "=" * 72)
    print("ALL SESSIONS")
    for k in ("REPLY", "GAP", "PACE"):
        print(summarise(k, everything[k]))


if __name__ == "__main__":
    main()
