import { cn } from "../lib/cn";
import { phaseBucket, PHASE_NAMES, type PhaseBucket, type PhaseName } from "../lib/phase";

// The IPAV set lives in `lib/phase.ts` (round 8) — one copy for the type,
// the select, and the tints.
export type Phase = PhaseName;
const PHASES: readonly Phase[] = PHASE_NAMES;

// Bucket -> pill accent classes. The phase->bucket mapping itself lives in
// `lib/phase.ts`, shared with SessionPhaseChip so the two widgets can't drift.
const pillTint: Record<PhaseBucket, string> = {
  primary: "border-primary/70 text-primary",
  secondary: "border-secondary/70 text-secondary",
  tertiary: "border-tertiary/70 text-tertiary",
};

const label: Record<Phase, string> = {
  investigate: "I",
  plan: "P",
  apply: "A",
  verify: "V",
};

interface PhasePillProps {
  phase: Phase;
  selected: boolean;
  onSelect: (p: Phase) => void;
}

export function PhasePill({ phase, selected, onSelect }: PhasePillProps) {
  return (
    <button
      type="button"
      role="tab"
      aria-selected={selected}
      onClick={() => onSelect(phase)}
      className={cn(
        "inline-flex items-center gap-1 rounded px-2 py-1 text-xs font-semibold uppercase",
        "border-t-2",
        // The accent is the SELECTED pill's (round 12). `cn` is clsx, not
        // tailwind-merge: with the tint and `text-on-surface-variant` both on
        // an unselected pill, whichever rule the stylesheet emits later won —
        // and that was the tint, so every unselected pill kept its phase
        // colour. Branching here leaves no pair to resolve.
        // `phase` is always one of the 4 IPAV phases, so the bucket is non-null.
        selected
          ? cn(pillTint[phaseBucket(phase)!], "bg-surface-container-high/80")
          : "bg-transparent border-transparent text-on-surface-variant hover:text-on-surface",
      )}
      title={phase}
    >
      <span>{label[phase]}</span>
    </button>
  );
}

export function PhasePillRow({
  selected,
  onSelect,
}: {
  // `null` = no phase highlighted (e.g. the sibling Tray tab is active).
  selected: Phase | null;
  onSelect: (p: Phase) => void;
}) {
  return (
    <div className="flex items-center gap-1">
      {PHASES.map((p) => (
        <PhasePill
          key={p}
          phase={p}
          selected={p === selected}
          onSelect={onSelect}
        />
      ))}
    </div>
  );
}
