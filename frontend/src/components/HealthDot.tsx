import { cn } from "../lib/cn";
import type { AgentHealth } from "../stores/health";

// Semantic status tokens: `success` (green) / `warning` (amber) / `error` (red).
const STYLES: Record<AgentHealth, { dot: string; label: string }> = {
  running: { dot: "bg-success", label: "running" },
  retrying: { dot: "bg-warning animate-pulse", label: "retrying (transient API error)" },
  stalled: { dot: "bg-error animate-pulse", label: "stalled — no response, possibly hung" },
  dead: { dot: "bg-error", label: "stopped — gave up after errors" },
};

/** A small colored liveness dot for an agent. `undefined` health is treated as
 *  running (the event only fires on transitions). `hideWhenHealthy` suppresses
 *  the dot while running — for low-noise problem-only indicators (dashboard). */
export function HealthDot({
  health,
  name,
  hideWhenHealthy = false,
}: {
  health?: AgentHealth;
  name: string;
  hideWhenHealthy?: boolean;
}) {
  const state: AgentHealth = health ?? "running";
  if (hideWhenHealthy && state === "running") return null;
  const s = STYLES[state];
  return (
    <span
      className={cn("inline-block size-2 shrink-0 rounded-full align-middle", s.dot)}
      title={`${name}: ${s.label}`}
      aria-label={`${name} ${s.label}`}
    />
  );
}

/** Session-level peer-forward router liveness. Binary + problem-only: renders a
 *  red pulsing dot ONLY when the router is DOWN (the common alive state shows
 *  nothing). `alive` undefined = assume alive (the event fires on transitions). */
export function RouterHealthDot({ alive }: { alive?: boolean }) {
  if (alive !== false) return null;
  const label = "peer-forward router DOWN — Brian↔Rain messages may not be delivered";
  return (
    <span
      className="inline-block size-2 shrink-0 rounded-full align-middle bg-error animate-pulse"
      title={label}
      aria-label={label}
    />
  );
}
