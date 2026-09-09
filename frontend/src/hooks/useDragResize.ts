import type { RefObject, MouseEvent as ReactMouseEvent } from "react";

/**
 * Column drag-resize. Returns an onMouseDown handler that tracks the pointer,
 * calls `compute(ev, rect)` for the new (already-clamped) value, persists it to
 * localStorage on release, and toggles the body cursor/userSelect for the drag.
 * The caller owns the value state and the clamp/units inside `compute`.
 */
export function useDragResize<E extends HTMLElement>(opts: {
  containerRef: RefObject<E>;
  value: number;
  setValue: (n: number) => void;
  storageKey: string;
  compute: (ev: MouseEvent, rect: DOMRect) => number;
}) {
  const { containerRef, value, setValue, storageKey, compute } = opts;
  return (e: ReactMouseEvent) => {
    e.preventDefault();
    const container = containerRef.current;
    if (!container) return;
    let latest = value;
    const onMove = (ev: MouseEvent) => {
      latest = compute(ev, container.getBoundingClientRect());
      setValue(latest);
    };
    const onUp = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      localStorage.setItem(storageKey, String(Math.round(latest)));
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
  };
}
