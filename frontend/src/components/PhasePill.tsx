import { cn } from "../lib/cn";
import { phaseBucket, PHASE_NAMES, type PhaseBucket, type PhaseName } from "../lib/phase";

// The IPAV set lives in `lib/phase.ts` (round 8) — one copy for the type,
// the select, and the tints.
export type Phase = PhaseName;
const PHASES: readonly Phase[] = PHASE_NAMES;

// Bucket -> the SELECTED pill's accent: its 2px underline and its text, in the
// phase's colour. The phase->bucket mapping itself lives in `lib/phase.ts`,
// shared with SessionPhaseChip so the two widgets can't drift.
//
// The pills are the app's one underline tab (`SubTabButton`'s style, the
// 2026-10-19 visual pass): a bottom border that is the accent when selected and
// transparent otherwise, no fill, mono label-caps text. The Tray and custom
// document tabs beside them in `DocumentPane` take the same style, with the
// primary accent where a phase pill has its own colour.
const pillTint: Record<PhaseBucket, string> = {
  primary: "border-primary text-primary",
  secondary: "border-secondary text-secondary",
  tertiary: "border-tertiary text-tertiary",
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
        "inline-flex items-center gap-1 border-b-2 px-2 py-1 font-label-caps text-label-caps uppercase transition-colors",
        // The accent is the SELECTED pill's (round 12). `cn` is clsx, not
        // tailwind-merge: with the tint and `text-on-surface-variant` both on
        // an unselected pill, whichever rule the stylesheet emits later won —
        // and that was the tint, so every unselected pill kept its phase
        // colour. Branching here leaves no pair to resolve.
        // `phase` is always one of the 4 IPAV phases, so the bucket is non-null.
        selected
          ? pillTint[phaseBucket(phase)!]
          : "border-transparent text-on-surface-variant hover:text-on-surface",
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
