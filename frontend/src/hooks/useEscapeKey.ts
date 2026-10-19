import { useEffect, useRef } from "react";

/** The overlays listening for Escape, oldest first. Escape answers only the
 *  NEWEST, so a dialog opened over another closes alone (EYES, once every
 *  modal shared `ui/Dialog`: with a listener each, one Escape closed both). */
const stack: { onEscape: () => void }[] = [];

function onKeyDown(e: KeyboardEvent) {
  if (e.key !== "Escape") return;
  const top = stack[stack.length - 1];
  if (!top) return;
  e.preventDefault();
  top.onEscape();
}

/**
 * Call `onClose` when Escape is pressed, while `enabled` (default true). Pass
 * the open/mounted flag as `enabled` so the listener is scoped to when the
 * overlay is actually showing. Shared by the dialogs/menus/drawers that all
 * hand-wired the same keydown effect.
 *
 * Overlays STACK: only the most recently enabled one hears Escape, and when it
 * closes the one beneath it hears the next press.
 */
export function useEscapeKey(onClose: () => void, enabled = true) {
  // The latest handler, so a re-render with a new closure neither re-orders
  // the stack nor calls a stale one.
  const latest = useRef(onClose);
  latest.current = onClose;
  useEffect(() => {
    if (!enabled) return;
    const entry = { onEscape: () => latest.current() };
    if (stack.length === 0) window.addEventListener("keydown", onKeyDown);
    stack.push(entry);
    return () => {
      const i = stack.indexOf(entry);
      if (i >= 0) stack.splice(i, 1);
      if (stack.length === 0) window.removeEventListener("keydown", onKeyDown);
    };
  }, [enabled]);
}
