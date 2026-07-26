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
}: {
  context?: AgentContext;
  name: string;
}) {
  const fraction = contextFraction(context);
  if (fraction === undefined || !context) return null;

  const severity = contextSeverity(fraction);
  // Round toward the user's interest: 99.6% should read "99%", not "100%",
  // because "100%" implies a wall that hasn't been hit yet.
  const pct = Math.min(100, Math.floor(fraction * 100));

  return (
    <span
      className={cn("ml-1 align-middle text-[0.6875rem] tabular-nums", TEXT[severity])}
      title={
        `${name}: ${formatTokens(context.usedTokens)} of ` +
        `${formatTokens(context.contextWindow)} context used (${pct}%), ` +
        `as of the last completed turn. This can go down — the agent ` +
        `auto-compacts when it fills up.`
      }
      aria-label={`${name} context ${pct} percent used`}
    >
      {pct}%
    </span>
  );
}
