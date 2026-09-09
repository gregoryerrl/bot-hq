import { useEffect, useRef } from "react";

/**
 * Focus management for modal dialogs. When `active`, on mount it focuses the
 * first focusable element inside the returned-ref container (falling back to the
 * container itself), traps Tab / Shift+Tab so focus can't escape to the page
 * behind the scrim, and restores focus to the previously-focused element on
 * close/unmount. Put the returned ref on the dialog's content container (give it
 * `tabIndex={-1}` so the container is a focus fallback).
 */
export function useFocusTrap<T extends HTMLElement>(active = true) {
  const ref = useRef<T>(null);
  useEffect(() => {
    if (!active) return;
    const container = ref.current;
    if (!container) return;
    const prevFocused = document.activeElement as HTMLElement | null;

    const focusables = () =>
      Array.from(
        container.querySelectorAll<HTMLElement>(
          'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])',
        ),
      ).filter((el) => el.offsetParent !== null);

    // Focus the first focusable (or the container) on open — unless something
    // inside already has focus (e.g. an `autoFocus` input).
    if (!container.contains(document.activeElement)) {
      (focusables()[0] ?? container).focus();
    }

    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Tab") return;
      const items = focusables();
      if (items.length === 0) {
        // Nothing tabbable — keep focus on the container.
        e.preventDefault();
        container.focus();
        return;
      }
      const first = items[0];
      const last = items[items.length - 1];
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    };
    container.addEventListener("keydown", onKey);
    return () => {
      container.removeEventListener("keydown", onKey);
      // Restore focus to the trigger so keyboard users land where they were.
      prevFocused?.focus?.();
    };
  }, [active]);

  return ref;
}
