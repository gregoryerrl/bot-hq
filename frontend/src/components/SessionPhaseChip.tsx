import { cn } from "../lib/cn";

export interface SessionPhaseChipProps {
  /** Raw phase string from `get_session_phase` (lowercased). Null = unknown / not live. */
  phase: string | null;
  /** True when the session row's `closed_at` is set — forces a DONE chip. */
  closed: boolean;
}

type Bucket = "primary" | "secondary" | "tertiary" | "muted";

const TINT: Record<Bucket, { bg: string; text: string; border: string }> = {
  primary: {
    bg: "bg-primary/15",
    text: "text-primary",
    border: "border-primary/30",
  },
  secondary: {
    bg: "bg-secondary/15",
    text: "text-secondary",
    border: "border-secondary/30",
  },
  tertiary: {
    bg: "bg-tertiary/15",
    text: "text-tertiary",
    border: "border-tertiary/30",
  },
  muted: {
    bg: "bg-outline-variant/15",
    text: "text-on-surface-variant",
    border: "border-outline-variant/30",
  },
};

function bucketFor(phase: string | null, closed: boolean): Bucket | null {
  if (closed) return "muted";
  if (!phase) return null;
  switch (phase.toLowerCase()) {
    case "investigate":
    case "plan":
      return "primary";
    case "apply":
      return "secondary";
    case "verify":
      return "tertiary";
    case "done":
      return "muted";
    default:
      return null;
  }
}

function labelFor(phase: string | null, closed: boolean): string | null {
  if (closed) return "DONE";
  if (!phase) return null;
  return phase.toUpperCase();
}

export function SessionPhaseChip({ phase, closed }: SessionPhaseChipProps) {
  const bucket = bucketFor(phase, closed);
  const label = labelFor(phase, closed);
  if (!bucket || !label) return null;
  const t = TINT[bucket];
  return (
    <span
      className={cn(
        "inline-flex items-center rounded border px-2 py-0.5 font-label-caps text-label-caps",
        t.bg,
        t.text,
        t.border,
      )}
    >
      {label}
    </span>
  );
}

/**
 * Internal helper exposed for tiles that need the phase's tint independently
 * (e.g., the accent bar). Returns Tailwind class strings or null when phase
 * is unknown.
 */
export function phaseTintClasses(
  phase: string | null,
  closed: boolean,
): { bar: string; ring: string } | null {
  const bucket = bucketFor(phase, closed);
  if (!bucket) return null;
  switch (bucket) {
    case "primary":
      return { bar: "bg-primary", ring: "border-primary/60" };
    case "secondary":
      return { bar: "bg-secondary", ring: "border-secondary/60" };
    case "tertiary":
      return { bar: "bg-tertiary", ring: "border-tertiary/60" };
    case "muted":
      return { bar: "bg-outline-variant", ring: "border-outline-variant" };
  }
}
