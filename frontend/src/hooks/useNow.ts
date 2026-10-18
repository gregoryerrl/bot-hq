import { useEffect, useState, useSyncExternalStore } from "react";

/**
 * The current time (epoch ms), re-read every `intervalMs` — for a render that
 * depends on a deadline nothing announces, such as a temporary halt's wake
 * time passing without a wake.
 */
export function useNow(intervalMs: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(id);
  }, [intervalMs]);
  return now;
}

/**
 * One shared clock for the many small "x ago" labels on a screen — a chat
 * row, a session tile, a feedback row. Every subscriber reads the same tick
 * (one interval for all of them, started with the first subscriber and
 * stopped with the last), so a hundred visible rows cost one timer, not a
 * hundred. Thirty seconds matches the labels' coarsest unit below an hour.
 */
const SHARED_TICK_MS = 30_000;
let sharedNow = Date.now();
let sharedTimer: ReturnType<typeof setInterval> | null = null;
const sharedListeners = new Set<() => void>();

function subscribeSharedNow(listener: () => void): () => void {
  sharedListeners.add(listener);
  if (sharedTimer === null) {
    sharedTimer = setInterval(() => {
      sharedNow = Date.now();
      for (const l of sharedListeners) l();
    }, SHARED_TICK_MS);
  }
  return () => {
    sharedListeners.delete(listener);
    if (sharedListeners.size === 0 && sharedTimer !== null) {
      clearInterval(sharedTimer);
      sharedTimer = null;
    }
  };
}

export function useSharedNow(): number {
  return useSyncExternalStore(subscribeSharedNow, () => sharedNow, () => sharedNow);
}
