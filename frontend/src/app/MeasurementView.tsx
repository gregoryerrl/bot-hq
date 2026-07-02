import { useTauriQuery } from "../hooks/useInvoke";
import { cn } from "../lib/cn";
import type { RetrievalStatsView } from "../lib/bindings";

// ============================================================================
// MeasurementView — Stage-4b CL retrieval telemetry (tokens-per-task et al.).
// Hosted under the Context Manager's "Measurement" pill, which carries the
// section label — so this renders content only, no heading of its own.
// ============================================================================

function StatTile({
  label,
  value,
  hint,
  accent,
  warn,
}: {
  label: string;
  value: string;
  hint?: string;
  accent?: boolean;
  warn?: boolean;
}) {
  return (
    <div
      className={cn(
        "rounded border border-outline-variant bg-surface-container-low px-3 py-2.5",
        accent && "border-primary/40 bg-primary/5",
      )}
    >
      <p className="font-label-caps text-label-caps text-on-surface-variant">
        {label}
      </p>
      <p
        className={cn(
          "mt-1 font-headline-md text-headline-md tabular-nums",
          warn ? "text-error" : accent ? "text-primary" : "text-on-surface",
        )}
      >
        {value}
      </p>
      {hint && (
        <p className="mt-0.5 font-code-sm text-code-sm text-on-surface-variant/60">
          {hint}
        </p>
      )}
    </div>
  );
}

export function MeasurementView({ project }: { project: string }) {
  const {
    data: stats,
    isFetching,
    error,
    refetch,
  } = useTauriQuery<RetrievalStatsView>("cl_retrieval_stats", {
    project,
    since: null,
  });

  const pct = (r: number) => `${(r * 100).toFixed(1)}%`;
  const num = (n: number) => Math.round(n).toLocaleString();

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-background">
      <div className="flex flex-shrink-0 items-center justify-between gap-3 border-b border-outline-variant px-4 py-1.5">
        <span className="font-code-sm text-code-sm text-on-surface-variant">
          cl_retrieve telemetry · all time
        </span>
        {/* Retrieval events emit no frontend event (deferred) — manual refresh. */}
        <button
          type="button"
          onClick={() => refetch()}
          className="rounded border border-outline-variant px-2 py-0.5 font-label-caps text-label-caps text-on-surface-variant hover:text-on-surface"
        >
          Refresh
        </button>
      </div>

      <div className="min-h-0 flex-1 overflow-auto px-4 py-4">
        {isFetching && !stats ? (
          <p className="font-code-sm text-code-sm text-on-surface-variant">
            Loading measurement…
          </p>
        ) : error ? (
          <p className="font-code-sm text-code-sm text-error">
            Failed to load measurement: {error.message}
          </p>
        ) : !stats || stats.event_count === 0 ? (
          <div className="flex h-full items-center justify-center text-center">
            <div>
              <p className="font-headline-md text-headline-md text-on-surface-variant">
                No retrievals logged yet
              </p>
              <p className="mt-1 font-code-sm text-code-sm text-on-surface-variant/60">
                Every cl_retrieve call an agent makes for {project} is logged
                here; the numbers appear once retrievals happen.
              </p>
            </div>
          </div>
        ) : (
          <div className="space-y-4">
            <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
              <StatTile
                label="Tokens / session"
                value={num(stats.avg_tokens_per_session)}
                hint="tokens-per-task proxy"
                accent
              />
              <StatTile
                label="Tokens / retrieval"
                value={num(stats.avg_tokens_per_event)}
              />
              <StatTile label="Retrievals" value={num(stats.event_count)} />
              <StatTile label="Sessions" value={num(stats.distinct_sessions)} />
              <StatTile
                label="Atoms returned"
                value={num(stats.total_atoms)}
              />
              <StatTile label="Total tokens" value={num(stats.total_tokens)} />
              <StatTile
                label="Stale-hit rate"
                value={pct(stats.stale_hit_rate)}
                hint={`${num(stats.stale_hits)} ⚠ atoms`}
                warn={stats.stale_hit_rate > 0}
              />
              <StatTile
                label="Retrieval-miss rate"
                value={pct(stats.empty_return_rate)}
                hint={`${num(stats.empty_returns)} empty`}
                warn={stats.empty_return_rate > 0}
              />
            </div>
            <p className="font-code-sm text-code-sm text-on-surface-variant/70">
              Low tokens/session with a low miss rate means retrieval is earning
              its keep. A rising stale-hit rate means atoms cite drifted code —
              time to re-index or prune.
            </p>
          </div>
        )}
      </div>
    </div>
  );
}
