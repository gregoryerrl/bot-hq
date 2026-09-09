import { useEffect } from "react";

/**
 * Call `onClose` when Escape is pressed, while `enabled` (default true). Pass
 * the open/mounted flag as `enabled` so the listener is scoped to when the
 * overlay is actually showing. Shared by the dialogs/menus/drawers that all
 * hand-wired the same keydown effect.
 */
export function useEscapeKey(onClose: () => void, enabled = true) {
  useEffect(() => {
    if (!enabled) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [enabled, onClose]);
}
