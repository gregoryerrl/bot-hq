import { useLayoutEffect, useRef, useState } from "react";

/**
 * Above this many characters a clamped recap gets a "show the full …"
 * toggle.
 *
 * **Set from what participants actually write, not from taste.** Across the 28
 * halts on record the reason averages 277 characters and the longest is 1,166 —
 * because the recap the user asked for is a recap: *"PR #514 is open, CI fully
 * green (ci 3m40s, quality 3m35s), mergeStateStatus CLEAN…"*. Rendered whole,
 * that is ~15 lines of banner sitting on top of the input box, and the banner
 * would push away the very box it exists to be adjacent to. A close card's
 * prompt is the same recap (feedback #73), so the gate clamps it the same way.
 */
export const RECAP_CLAMP_CHARS = 200;

/**
 * Would a `line-clamp-N` visually cut this text? The static floor.
 *
 * Char count alone lied (issues.md 2026-08-24): the clamp is a line clamp, so
 * a SUB-200-char reason with more newlines than lines — the bulleted shape
 * agents actually write — rendered truncated with NO toggle at all, and the
 * rest of the recap was unreachable by any interaction. Rendered line count
 * and char count are different quantities; this gates the toggle on either.
 * `lines` is the clamp's line count: `newlines >= lines` means one line past
 * it.
 */
export function clampOverflows(text: string, lines = 3): boolean {
  const newlines = (text.match(/\n/g) ?? []).length;
  return text.length > RECAP_CLAMP_CHARS || newlines >= lines;
}

/**
 * The RENDERED half of the overflow question (EYES B1, advisory 74113c5f).
 *
 * {@link clampOverflows} is a static floor, and any static rule is a proxy:
 * the left panel is user-resizable, so a 190-char single-line recap wraps to
 * 5 lines in a narrow pane, clamps at 3, and the static gate shows no toggle
 * — the user's filed bug back again through a different door. Layout is the
 * only authority on layout: while the text is COLLAPSED, compare
 * `scrollHeight` to `clientHeight` on the clamped element and re-measure on
 * resize. The measurement can only ADD the toggle, never remove it, and it
 * keeps its last value while expanded so "show less" cannot vanish. In jsdom
 * both heights are 0, so tests exercise the static floor; the measured path
 * is covered by mocking the height getters.
 */
export function useMeasuredClampOverflow<T extends HTMLElement>(
  text: string,
  expanded: boolean,
): [React.RefObject<T>, boolean] {
  const ref = useRef<T>(null);
  const [overflows, setOverflows] = useState(false);
  useLayoutEffect(() => {
    const el = ref.current;
    // Measure the CLAMPED rendering only — expanded, scrollHeight measures
    // the internal scroller instead, which says nothing about the clamp.
    if (!el || expanded) return;
    // +1: fractional line-height rounding must not flicker the toggle.
    const measure = () => setOverflows(el.scrollHeight > el.clientHeight + 1);
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [text, expanded]);
  return [ref, overflows];
}
