"""Load SWE-bench Verified instances without the heavy `datasets` library.

Uses the HuggingFace datasets-server REST API (stdlib urllib) so a smoke needs
no pip install. Caveat: the rows API can truncate very large cells — we count
and warn. For the full 500-instance run prefer the `datasets` library for full
fidelity (see README / requirements-full.txt).
"""
from __future__ import annotations

import json
import time
import urllib.parse
import urllib.request

_DATASET = "princeton-nlp/SWE-bench_Verified"
_ROWS = "https://datasets-server.huggingface.co/rows"
_SPLITS = "https://datasets-server.huggingface.co/splits"
_PAGE = 100  # rows-API max length per request


def _get(url: str) -> dict:
    req = urllib.request.Request(url, headers={"Accept": "application/json"})
    last = None
    for attempt in range(4):  # the HF datasets-server occasionally times out
        try:
            with urllib.request.urlopen(req, timeout=60) as r:
                return json.loads(r.read())
        except Exception as e:  # noqa: BLE001 — retry transient timeout / 5xx
            last = e
            time.sleep(2 * (attempt + 1))
    raise last


def _discover() -> tuple[str, str]:
    """Resolve (config, split) names rather than guessing 'default'/'test'."""
    data = _get(f"{_SPLITS}?{urllib.parse.urlencode({'dataset': _DATASET})}")
    splits = data.get("splits", [])
    for s in splits:
        if s.get("split") == "test":
            return s["config"], s["split"]
    if splits:
        return splits[0]["config"], splits[0]["split"]
    raise RuntimeError(f"no splits found for {_DATASET}")


def load_by_ids(ids: list[str]) -> tuple[list[dict], int]:
    """Return (instance dicts matching `ids`, truncated count), preserving input order.

    Scans the dataset pages until all requested ids are found. Lets the harness
    run a hand-picked diverse set (e.g. N per repo) instead of a contiguous window.
    """
    want = [i for i in ids if i]
    wantset = set(want)
    config, split = _discover()
    found: dict[str, dict] = {}
    truncated = 0
    offset = 0
    while len(found) < len(wantset):
        qs = urllib.parse.urlencode({
            "dataset": _DATASET, "config": config, "split": split,
            "offset": offset, "length": _PAGE,
        })
        rows = _get(f"{_ROWS}?{qs}").get("rows", [])
        if not rows:
            break
        for entry in rows:
            row = entry.get("row", {})
            if row.get("instance_id") in wantset:
                found[row["instance_id"]] = row
                if entry.get("truncated_cells"):
                    truncated += 1
        offset += len(rows)
        if len(rows) < _PAGE:
            break
    return [found[i] for i in want if i in found], truncated


def load_instances(n: int, offset: int = 0) -> tuple[list[dict], int]:
    """Return (up to n instance dicts, count of rows with truncated cells)."""
    config, split = _discover()
    out: list[dict] = []
    truncated = 0
    got = 0
    while got < n:
        length = min(_PAGE, n - got)
        qs = urllib.parse.urlencode({
            "dataset": _DATASET, "config": config, "split": split,
            "offset": offset + got, "length": length,
        })
        rows = _get(f"{_ROWS}?{qs}").get("rows", [])
        if not rows:
            break
        for entry in rows:
            out.append(entry.get("row", {}))
            if entry.get("truncated_cells"):
                truncated += 1
        got += len(rows)
        if len(rows) < length:
            break
    return out, truncated
