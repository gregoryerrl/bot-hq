import { useEffect, useState } from "react";

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
