import { cn } from "../lib/cn";

export type Phase = "investigate" | "plan" | "apply" | "verify";
const PHASES: Phase[] = ["investigate", "plan", "apply", "verify"];

const tintByPhase: Record<Phase, string> = {
  investigate: "border-amber-500/70 text-amber-300",
  plan: "border-blue-500/70 text-blue-300",
  apply: "border-emerald-500/70 text-emerald-300",
  verify: "border-purple-500/70 text-purple-300",
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
          ? "bg-neutral-800/80"
          : "bg-transparent border-transparent text-neutral-500 hover:text-neutral-300",
      )}
      title={phase}
    >
      <span>{label[phase]}</span>
      {count !== undefined && count > 0 && (
        <span className="text-[0.65rem] text-neutral-400">·{count}</span>
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
