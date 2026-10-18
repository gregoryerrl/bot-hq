import { useSharedNow } from "../hooks/useNow";
import { formatRelative, formatTimestamp, parseUtcMs } from "../lib/time";

/**
 * "4m ago" that keeps counting, with the exact local time on hover.
 *
 * `formatRelative` alone froze: a chat row is memoised and a tile re-renders
 * only on new data, so an idle session read "just now" for an hour. One
 * shared 30 s tick re-renders every visible label; the absolute time rides
 * the `title` (and `dateTime`), which nothing on screen showed before.
 */
export function RelativeTime({
  iso,
  fallback = "",
  className,
}: {
  iso: string;
  /** Shown when the timestamp is empty or unreadable. */
  fallback?: string;
  className?: string;
}) {
  useSharedNow();
  const label = formatRelative(iso) || fallback;
  const ms = parseUtcMs(iso);
  const absolute = Number.isFinite(ms) ? formatTimestamp(iso) : undefined;
  return (
    <time
      dateTime={Number.isFinite(ms) ? new Date(ms).toISOString() : undefined}
      title={absolute}
      className={className}
    >
      {label}
    </time>
  );
}
