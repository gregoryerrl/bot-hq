// Single source of the IPAV phase -> color-bucket mapping. Both phase widgets
// derive their tints from this — PhasePill (the I/P/A/V tab selector) and
// SessionPhaseChip (the dashboard status chip) — so the two stay in agreement
// structurally instead of via a hand-synced comment.

export type PhaseBucket = "primary" | "secondary" | "tertiary";

/**
 * Map a phase string to its color bucket. Accepts any-case input (the chip reads
 * a raw `get_session_phase` string) and returns null for unknown phases. The
 * "done" / closed -> muted handling stays with SessionPhaseChip, since it's
 * chip-only state, not a phase color.
 */
export function phaseBucket(phase: string): PhaseBucket | null {
  switch (phase.toLowerCase()) {
    case "investigate":
    case "plan":
      return "primary";
    case "apply":
      return "secondary";
    case "verify":
      return "tertiary";
    default:
      return null;
  }
}
