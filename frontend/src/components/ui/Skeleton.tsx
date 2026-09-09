import { cn } from "../../lib/cn";

/**
 * Loading placeholder rows — one pulse animation, one shape per call site.
 * Five panels each spelled `{[0,1,2].map(i => <div key={i} className="h-N
 * animate-pulse rounded… bg-…"/>)}` by hand with drifted heights and radii
 * (round 11); the height and surface still belong to the caller (a session
 * tile is not a violations row), the idiom does not.
 */
export function Skeleton({
  rows = 3,
  className,
  rowClassName,
}: {
  /** How many placeholder rows. */
  rows?: number;
  /** The container — spacing/grid classes. */
  className?: string;
  /** One row — its height, radius and surface. */
  rowClassName: string;
}) {
  return (
    <div className={className} aria-busy="true" aria-hidden="true">
      {Array.from({ length: rows }, (_, i) => (
        <div key={i} className={cn("animate-pulse", rowClassName)} />
      ))}
    </div>
  );
}
