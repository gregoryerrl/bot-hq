import { useCallback, useEffect, useRef, useState } from "react";

/** Movement before a press becomes a drag — below it, it stays a click. */
export const DRAG_THRESHOLD_PX = 6;

/**
 * Drag one card onto another to swap them, on POINTER events.
 *
 * Not HTML5 drag-and-drop: Tauri keeps `dragDropEnabled` on so the composer
 * can take files dropped from the OS with their real paths, and with it on the
 * webview never fires an HTML5 `drop`. The dashboard's swap was built on that
 * event and could never complete in the app, while its jsdom tests passed on
 * synthetic drops (the user, 2026-09-25: "I'm not able to swap the cards").
 *
 * A press becomes a drag after `DRAG_THRESHOLD_PX` of movement, tracked at
 * the WINDOW so leaving the card does not end it. The card under the pointer
 * is the element carrying `attr`. Releasing over a different card swaps; text
 * selection is off while dragging; a cancel or a lost window focus ends the
 * drag without a swap. The click that follows a drag is swallowed, so a drag
 * never opens the card — a plain click still does.
 */
export function usePointerSwap(
  onSwap: (from: string, to: string) => void,
  attr = "data-session-tile",
) {
  const [dragId, setDragId] = useState<string | null>(null);
  const [dropTarget, setDropTarget] = useState<string | null>(null);
  const suppressClick = useRef(false);
  const cleanupRef = useRef<(() => void) | null>(null);
  useEffect(() => () => cleanupRef.current?.(), []);

  const onPointerDown = useCallback(
    (e: { button: number; clientX: number; clientY: number }, id: string) => {
      if (e.button !== 0) return;
      suppressClick.current = false;
      cleanupRef.current?.();
      const startX = e.clientX;
      const startY = e.clientY;
      let active = false;
      const cardAt = (x: number, y: number): string | null => {
        const hit = document.elementFromPoint(x, y)?.closest(`[${attr}]`);
        const card = hit?.getAttribute(attr) ?? null;
        return card && card !== id ? card : null;
      };
      const cleanup = () => {
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", up);
        window.removeEventListener("pointercancel", cancel);
        window.removeEventListener("blur", cancel);
        document.body.style.userSelect = "";
        setDragId(null);
        setDropTarget(null);
        cleanupRef.current = null;
      };
      const move = (ev: PointerEvent) => {
        if (!active) {
          if (Math.hypot(ev.clientX - startX, ev.clientY - startY) < DRAG_THRESHOLD_PX) return;
          active = true;
          setDragId(id);
          document.body.style.userSelect = "none";
        }
        setDropTarget(cardAt(ev.clientX, ev.clientY));
      };
      const up = (ev: PointerEvent) => {
        const wasDrag = active;
        const target = wasDrag ? cardAt(ev.clientX, ev.clientY) : null;
        cleanup();
        if (!wasDrag) return;
        // The click that follows this release must not open the card. It
        // fires in this same turn of the event loop, so the flag clears after.
        suppressClick.current = true;
        setTimeout(() => {
          suppressClick.current = false;
        }, 0);
        if (target) onSwap(id, target);
      };
      const cancel = () => cleanup();
      cleanupRef.current = cleanup;
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", up);
      window.addEventListener("pointercancel", cancel);
      window.addEventListener("blur", cancel);
    },
    [onSwap, attr],
  );

  const onClickCapture = useCallback((e: { stopPropagation(): void; preventDefault(): void }) => {
    if (!suppressClick.current) return;
    suppressClick.current = false;
    e.stopPropagation();
    e.preventDefault();
  }, []);

  return { dragId, dropTarget, onPointerDown, onClickCapture };
}
