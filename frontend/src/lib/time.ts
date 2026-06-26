/**
 * Time helpers. Every timestamp the backend stores is RFC3339 in UTC
 * (`2026-06-03T07:40:00.123Z`). UTC is the single baseline; we only convert to
 * the viewer's local zone at render time.
 *
 * `parseUtcMs` is the load-bearing piece: legacy rows (and any SQLite-native
 * write) can still be ZONE-LESS (`2026-06-03 07:40:00`), which `new Date(iso)`
 * misparses as LOCAL — inflating "x ago" by the viewer's UTC offset (the
 * "stale 8h" hallucination). We detect a missing zone and pin it to UTC.
 */

const HAS_ZONE = /([zZ])$|[+-]\d{2}:?\d{2}$/;

/** Parse a stored timestamp to epoch ms, treating zone-less strings as UTC. */
export function parseUtcMs(iso: string): number {
  if (!iso) return NaN;
  let s = iso.trim();
  if (!HAS_ZONE.test(s)) {
    // SQLite "YYYY-MM-DD HH:MM:SS" → RFC3339 UTC.
    s = s.replace(" ", "T") + "Z";
  }
  return new Date(s).getTime();
}

/** Relative "x ago" label. Zone-safe via {@link parseUtcMs}. */
export function formatRelative(iso: string): string {
  if (!iso) return "";
  const then = parseUtcMs(iso);
  if (!Number.isFinite(then)) return "";
  const sec = Math.max(0, Math.floor((Date.now() - then) / 1000));
  if (sec < 5) return "just now";
  if (sec < 60) return `${sec}s ago`;
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const day = Math.floor(hr / 24);
  return `${day}d ago`;
}

/** Absolute local date-time label. Zone-safe via {@link parseUtcMs}. */
export function formatTimestamp(iso: string): string {
  if (!iso) return "";
  const ms = parseUtcMs(iso);
  if (!Number.isFinite(ms)) return iso;
  return new Date(ms).toLocaleString();
}
