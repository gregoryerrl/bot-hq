import type { ReactNode } from "react";
import type { GatedKeyword, GateMode } from "../lib/bindings";
import { SegToggle } from "./ui/SegToggle";
import { useListEditor } from "../hooks/useListEditor";

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
  const { keys, replaceAt, removeAt, append } = useListEditor(value, onChange);
  const updateRow = (i: number, patch: Partial<GatedKeyword>) =>
    replaceAt(i, { ...value[i], ...patch });
  const addRow = () => append({ keyword: "", mode: "gate" });
  // "Set all" (2026-08-27, user request): flipping a long keyword list
  // one row at a time is the friction that pushed a whole session to
  // auto-allow by hand. One emit, every row, same onChange contract.
  const setAll = (mode: GateMode) =>
    onChange(value.map((k) => ({ ...k, mode })));

  return (
    <>
      {value.length > 1 && (
        <div className="mb-2 flex items-center gap-2">
          <span className="font-label-caps text-label-caps text-on-surface-variant">
            Set all
          </span>
          <div className="flex overflow-hidden rounded border border-outline-variant">
            <button
              type="button"
              onClick={() => setAll("gate")}
              className="px-2 py-1 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:bg-primary/15 hover:text-primary"
            >
              Gate
            </button>
            <button
              type="button"
              onClick={() => setAll("auto_allow")}
              className="border-l border-outline-variant px-2 py-1 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:bg-success/15 hover:text-success"
            >
              Auto-allow
            </button>
          </div>
        </div>
      )}
      {value.length === 0 ? (
        emptyState
      ) : (
        <ul className="flex flex-col gap-2">
          {value.map((k, i) => (
            <li key={keys[i]} className="flex items-center gap-2">
              <input
                type="text"
                value={k.keyword}
                onChange={(e) => updateRow(i, { keyword: e.target.value })}
                placeholder={placeholder}
                className={inputClassName}
              />
              <SegToggle<GateMode>
                value={k.mode}
                onChange={(mode) => updateRow(i, { mode })}
                className="shrink-0"
                options={[
                  { value: "gate", label: "Gate", tone: "warn" },
                  { value: "auto_allow", label: "Auto-allow", tone: "good" },
                ]}
              />
              <button
                type="button"
                onClick={() => removeAt(i)}
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
