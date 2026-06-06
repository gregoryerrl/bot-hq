import { cn } from "../lib/cn";
import { phaseBucket, type PhaseBucket } from "../lib/phase";

export type Phase = "investigate" | "plan" | "apply" | "verify";
const PHASES: Phase[] = ["investigate", "plan", "apply", "verify"];

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
      onClick={() => onSelect(phase)}
      className={cn(
        "inline-flex items-center gap-1 rounded px-2 py-1 text-xs font-semibold uppercase",
        "border-t-2",
        // `phase` is always one of the 4 IPAV phases, so the bucket is non-null.
        pillTint[phaseBucket(phase)!],
        selected
          ? "bg-surface-container-high/80"
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
