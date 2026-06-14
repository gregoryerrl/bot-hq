import { cn } from "../lib/cn";
import type { AgentHealth } from "../stores/health";

// No semantic success/warn tokens exist yet — emerald/amber raw are the
// house stand-ins (see SessionPolicyPanel / PluginManager). `error` IS a token.
const STYLES: Record<AgentHealth, { dot: string; label: string }> = {
  running: { dot: "bg-emerald-400", label: "running" },
  retrying: { dot: "bg-amber-400 animate-pulse", label: "retrying (transient API error)" },
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
