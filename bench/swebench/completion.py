"""Pure completion-detection + choice-auto-pick logic for the rollout loop.

No I/O — fully unit-testable. The subtle rule: `request_approval` parks a
choice AND sets awaiting=true, while `mark_awaiting_user("SWEBENCH_DONE")` sets
awaiting=true with NO pending choice. So completion = awaiting AND no pending
choice; a pending choice means it's a gate to auto-resolve, not completion.
"""
from __future__ import annotations

import re
from dataclasses import dataclass

# Options that mean "keep going" when we have to auto-answer a headless gate.
_PROCEED_RE = re.compile(r"\b(proceed|continue|yes|approve|approved|go ahead|confirm|ok|okay)\b", re.I)


def pick_choice_option(options: list[str]) -> str:
    """Auto-answer a parked ask_user_choice / request_approval in a no-human run.

    Prefer an option that means 'keep going'; otherwise fall back to the first.
    """
    if not options:
        return "proceed"
    for opt in options:
        if _PROCEED_RE.search(opt or ""):
            return opt
    return options[0]


def max_id(messages: list[dict], current: int) -> int:
    """Largest message id seen so far (the external API returns no max_id)."""
    for m in messages:
        mid = m.get("id")
        if isinstance(mid, int) and mid > current:
            current = mid
    return current


@dataclass
class Decision:
    done: bool
    reason: str


def evaluate(
    *,
    awaiting: bool,
    pending_choice_count: int,
    seconds_since_last_msg: float,
    elapsed: float,
    silence_timeout: float,
    wall_clock_cap: float,
) -> Decision:
    """Decide whether this instance's rollout is finished.

    Priority: hard wall-clock cap > completion sentinel > silence backstop.
    """
    if elapsed >= wall_clock_cap:
        return Decision(True, f"wall-clock cap {wall_clock_cap:.0f}s reached")
    if awaiting and pending_choice_count == 0:
        return Decision(True, "agent signaled completion (awaiting flag, no pending choice)")
    if seconds_since_last_msg >= silence_timeout:
        return Decision(True, f"message silence {seconds_since_last_msg:.0f}s >= {silence_timeout:.0f}s")
    return Decision(False, "in progress")
