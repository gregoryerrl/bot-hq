import { cn } from "../lib/cn";

export type Phase = "investigate" | "plan" | "apply" | "verify";
const PHASES: Phase[] = ["investigate", "plan", "apply", "verify"];

// Phase->color buckets match SessionPhaseChip (investigate/plan=primary,
// apply=secondary, verify=tertiary) so the two IPAV widgets agree.
const tintByPhase: Record<Phase, string> = {
  investigate: "border-primary/70 text-primary",
  plan: "border-primary/70 text-primary",
  apply: "border-secondary/70 text-secondary",
  verify: "border-tertiary/70 text-tertiary",
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
  count?: number;
}

export function PhasePill({ phase, selected, onSelect, count }: PhasePillProps) {
  return (
    <button
      onClick={() => onSelect(phase)}
      className={cn(
        "inline-flex items-center gap-1 rounded px-2 py-1 text-xs font-semibold uppercase",
        "border-t-2",
        tintByPhase[phase],
        selected
          ? "bg-surface-container-high/80"
          : "bg-transparent border-transparent text-on-surface-variant hover:text-on-surface",
      )}
      title={phase}
    >
      <span>{label[phase]}</span>
      {count !== undefined && count > 0 && (
        <span className="text-[0.65rem] text-on-surface-variant">·{count}</span>
      )}
    </button>
  );
}

export function PhasePillRow({
  selected,
  onSelect,
  counts,
}: {
  selected: Phase;
  onSelect: (p: Phase) => void;
  counts?: Partial<Record<Phase, number>>;
}) {
  return (
    <div className="flex items-center gap-1">
      {PHASES.map((p) => (
        <PhasePill
          key={p}
          phase={p}
          selected={p === selected}
          onSelect={onSelect}
          count={counts?.[p]}
        />
      ))}
    </div>
  );
}
