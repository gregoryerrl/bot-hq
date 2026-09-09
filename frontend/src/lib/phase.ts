// Single source of the IPAV phase -> color-bucket mapping. Both phase widgets
// derive their tints from this — PhasePill (the I/P/A/V tab selector) and
// SessionPhaseChip (the dashboard status chip) — so the two stay in agreement
// structurally instead of via a hand-synced comment.

export type PhaseBucket = "primary" | "secondary" | "tertiary";

/**
 * The IPAV phases, in order — the ONE frontend copy of `core/ipav.rs`'s set.
 * `SessionView`'s phase select and `phaseBucket` below both derive from it
 * (round 8): they used to be two hand-written lists, so a renamed phase could
 * stop matching in one of them silently (an untinted chip, a missing option).
 */
export const PHASE_NAMES = ["investigate", "plan", "apply", "verify"] as const;
export type PhaseName = (typeof PHASE_NAMES)[number];

/** Is this (any-case) string one of the IPAV phase names? */
export function isPhaseName(s: string): s is PhaseName {
  return (PHASE_NAMES as readonly string[]).includes(s.toLowerCase());
}

/**
 * Map a phase string to its color bucket. Accepts any-case input (the chip reads
 * a raw `get_session_phase` string) and returns null for unknown phases. The
 * "done" / closed -> muted handling stays with SessionPhaseChip, since it's
 * chip-only state, not a phase color.
 */
export function phaseBucket(phase: string): PhaseBucket | null {
  const lower = phase.toLowerCase();
  if (!isPhaseName(lower)) return null;
  const buckets: Record<PhaseName, PhaseBucket> = {
    investigate: "primary",
    plan: "primary",
    apply: "secondary",
    verify: "tertiary",
  };
  return buckets[lower];
}
