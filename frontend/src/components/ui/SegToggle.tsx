import { cn } from "../../lib/cn";

export type SegTone = "good" | "warn" | "danger";

/**
 * Segmented two-or-more-button toggle. Each option declares its active `tone`
 * (good=success green, warn=primary, danger=error). Shared by the policy form,
 * the Tool-Gate keyword rows, and the Context Library Form/Raw switcher.
 */
export function SegToggle<T extends string>({
  value,
  options,
  onChange,
  disabled,
  className,
}: {
  value: T;
  options: { value: T; label: string; tone: SegTone }[];
  onChange: (v: T) => void;
  disabled?: boolean;
  /** Extra classes on the wrapper (e.g. `shrink-0` inside a flex row). */
  className?: string;
}) {
  return (
    <div
      className={cn(
        "flex overflow-hidden rounded border border-outline-variant",
        className,
      )}
    >
      {options.map((o) => {
        const active = value === o.value;
        const activeCls =
          o.tone === "good"
            ? "bg-success/15 text-success"
            : o.tone === "warn"
              ? "bg-primary/15 text-primary"
              : "bg-error/20 text-on-error-container";
        return (
          <button
            key={o.value}
            type="button"
            disabled={disabled}
            onClick={() => onChange(o.value)}
            className={cn(
              "px-3 py-1 font-label-caps text-label-caps transition-colors disabled:opacity-50",
              active
                ? activeCls
                : "bg-transparent text-on-surface-variant hover:text-on-surface",
            )}
          >
            {o.label}
          </button>
        );
      })}
    </div>
  );
}
