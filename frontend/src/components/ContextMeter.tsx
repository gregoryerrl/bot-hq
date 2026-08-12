import { cn } from "../lib/cn";
import {
  contextFraction,
  contextSeverity,
  formatTokens,
  type AgentContext,
} from "../stores/context";

const TEXT: Record<"ok" | "warn" | "critical", string> = {
  ok: "text-on-surface-variant",
  warn: "text-warning",
  critical: "text-error",
};

/** Per-agent context-window occupancy, sized to sit inline beside a HealthDot.
 *
 *  Renders nothing at all when occupancy is unknown — an agent that hasn't
 *  finished a turn yet, or one on a provider that doesn't report a window
 *  (the DeepSeek gateway may be such a case). Showing "0%" there would be a
 *  confident lie about a number we do not have; an absent badge is honest.
 *
 *  The value is **as of the last completed turn** and is **non-monotonic** —
 *  claude-code auto-compacts, which makes it drop. Both facts are surfaced in
 *  the tooltip rather than hidden, so a user who sees it fall doesn't read it
 *  as a bug. */
export function ContextMeter({
  context,
  name,
  onOpenHistory,
}: {
  context?: AgentContext;
  name: string;
  /** Open this participant's recorded readings (rc3 P7). */
  onOpenHistory?: () => void;
}) {
  const fraction = contextFraction(context);
  const known = fraction !== undefined && context !== undefined;

  // No live reading. The badge still renders — as `ctx`, never as a number —
  // because the recorded history behind it is exactly what this state needs
  // explaining (rc3 P7): a participant whose provider sends no `contextWindow`
  // shows no meter right up until it dies of a prompt that is too long, and
  // before this there was nothing to click and nothing written down. `ctx` is
  // an affordance, not a measurement, so the no-confident-lie rule holds.
  if (!known) {
    if (!onOpenHistory) return null;
    return (
      <button
        type="button"
        onClick={onOpenHistory}
        className="ml-1 align-middle text-[0.6875rem] text-outline underline decoration-dotted underline-offset-2 transition-colors hover:text-on-surface-variant"
        title={
          `${name}: no context reading yet — the agent hasn't finished a turn, ` +
          `or its provider reports no context window. Open the recorded ` +
          `readings to see which.`
        }
        aria-label={`${name} context history`}
      >
        ctx
      </button>
    );
  }

  const severity = contextSeverity(fraction);
  // Round toward the user's interest: 99.6% should read "99%", not "100%",
  // because "100%" implies a wall that hasn't been hit yet.
  const pct = Math.min(100, Math.floor(fraction * 100));
  const title =
    `${name}: ${formatTokens(context.usedTokens)} of ` +
    `${formatTokens(context.contextWindow)} context used (${pct}%), ` +
    `as of the last completed turn. This can go down — the agent ` +
    `auto-compacts when it fills up.` +
    (onOpenHistory ? " Click for the recorded readings." : "");
  const className = cn(
    "ml-1 align-middle text-[0.6875rem] tabular-nums",
    TEXT[severity],
  );

  if (!onOpenHistory) {
    return (
      <span
        className={className}
        title={title}
        aria-label={`${name} context ${pct} percent used`}
      >
        {pct}%
      </span>
    );
  }
  return (
    <button
      type="button"
      onClick={onOpenHistory}
      className={cn(className, "underline decoration-dotted underline-offset-2")}
      title={title}
      aria-label={`${name} context ${pct} percent used`}
    >
      {pct}%
    </button>
  );
}
