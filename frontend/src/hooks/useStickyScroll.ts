import { useCallback, useEffect, useRef, useState } from "react";

const STICKY_THRESHOLD_PX = 80;

/**
 * Chat-style auto-scroll. Tracks whether the user is "at the bottom" of a
 * scroll container; when new content arrives, auto-scroll only if they were
 * already there. Exposes `stuck` and a `scrollToBottom()` helper so the UI
 * can show a "↓ N new" button when the user has scrolled up.
 *
 * Usage:
 *
 *   const { ref, stuck, scrollToBottom } = useStickyScroll([messages.length]);
 *   <div ref={ref}>…</div>
 */
export function useStickyScroll<T extends HTMLElement>(
  deps: React.DependencyList,
) {
  const ref = useRef<T>(null);
  const [stuck, setStuck] = useState(true);

  const recomputeStuck = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
    setStuck(distanceFromBottom < STICKY_THRESHOLD_PX);
  }, []);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const onScroll = () => recomputeStuck();
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => el.removeEventListener("scroll", onScroll);
  }, [recomputeStuck]);

  // Auto-scroll on deps change IF stuck.
  useEffect(() => {
    const el = ref.current;
    if (!el || !stuck) return;
    // Defer to next frame so layout has settled.
    requestAnimationFrame(() => {
      el.scrollTop = el.scrollHeight;
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);

  const scrollToBottom = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
    setStuck(true);
  }, []);

  return { ref, stuck, scrollToBottom };
}
