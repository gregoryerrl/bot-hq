import type { ReactNode } from "react";
import type { GatedKeyword, GateMode } from "../lib/bindings";
import { cn } from "../lib/cn";

const GATE_MODES: GateMode[] = ["gate", "auto_allow"];

interface GatedKeywordListProps {
  /** Current keyword rows (the caller's draft). */
  value: GatedKeyword[];
  /** Emit the next list on any row add/edit/remove. */
  onChange: (next: GatedKeyword[]) => void;
  /** className for each keyword text input — styling differs per surface. */
  inputClassName?: string;
  /** Placeholder for the keyword text input. */
  placeholder?: string;
  /** Rendered in place of the list when there are no rows. */
  emptyState: ReactNode;
  /** Caller-supplied add control + any chrome (dirty/saved indicators). */
  footer: (addRow: () => void) => ReactNode;
}

/**
 * Editable list of Tool Gate keyword rows: a text input + a Gate/Auto-allow
 * toggle pair + a remove button per row. Presentational — the caller owns the
 * fetch/save/dirty wiring and passes `value`/`onChange`. Shared by Settings
 * (global Tool Gate) and SessionPolicyPanel (per-session gate); the surfaces
 * differ only in input styling, empty copy, and the add-button chrome, which
 * the props above parameterize.
 */
export function GatedKeywordList({
  value,
  onChange,
  inputClassName,
  placeholder = "keyword (e.g. gh issue, git push, curl)",
  emptyState,
  footer,
}: GatedKeywordListProps) {
  const updateRow = (i: number, patch: Partial<GatedKeyword>) =>
    onChange(value.map((k, idx) => (idx === i ? { ...k, ...patch } : k)));
  const removeRow = (i: number) =>
    onChange(value.filter((_, idx) => idx !== i));
  const addRow = () => onChange([...value, { keyword: "", mode: "gate" }]);

  return (
    <>
      {value.length === 0 ? (
        emptyState
      ) : (
        <ul className="flex flex-col gap-2">
          {value.map((k, i) => (
            <li key={i} className="flex items-center gap-2">
              <input
                type="text"
                value={k.keyword}
                onChange={(e) => updateRow(i, { keyword: e.target.value })}
                placeholder={placeholder}
                className={inputClassName}
              />
              <div className="flex shrink-0 overflow-hidden rounded border border-outline-variant">
                {GATE_MODES.map((m) => {
                  const active = k.mode === m;
                  const activeCls =
                    m === "gate"
                      ? "bg-primary/15 text-primary"
                      : "bg-success/15 text-success";
                  return (
                    <button
                      key={m}
                      type="button"
                      onClick={() => updateRow(i, { mode: m })}
                      className={cn(
                        "px-2.5 py-1 font-label-caps text-label-caps transition-colors",
                        active
                          ? activeCls
                          : "bg-transparent text-on-surface-variant hover:text-on-surface",
                      )}
                    >
                      {m === "gate" ? "Gate" : "Auto-allow"}
                    </button>
                  );
                })}
              </div>
              <button
                type="button"
                onClick={() => removeRow(i)}
                aria-label="Remove keyword"
                className="shrink-0 rounded border border-outline-variant bg-transparent px-2 py-1 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:bg-surface-container-high hover:text-on-surface"
              >
                ✕
              </button>
            </li>
          ))}
        </ul>
      )}
      {footer(addRow)}
    </>
  );
}
