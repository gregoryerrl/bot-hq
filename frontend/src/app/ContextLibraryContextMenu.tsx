import { useEffect, useRef, useState } from "react";
import { cn } from "../lib/cn";
import { terminalInputClass } from "./contextLibraryShared";

// ============================================================================
// ContextMenu — VSCode-style right-click menu, fixed at the cursor. Closes on
// outside-click or Escape.
// ============================================================================

export interface ContextMenuItem {
  label: string;
  onSelect: () => void;
  danger?: boolean;
}

export function ContextMenu({
  x,
  y,
  items,
  onClose,
}: {
  x: number;
  y: number;
  items: ContextMenuItem[];
  onClose: () => void;
}) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [onClose]);

  return (
    <div
      ref={ref}
      role="menu"
      className="fixed z-50 min-w-[160px] rounded border border-outline-variant bg-surface-container-high py-1 shadow-lg"
      style={{ top: y, left: x }}
    >
      {items.map((it) => (
        <button
          key={it.label}
          type="button"
          role="menuitem"
          onClick={() => {
            it.onSelect();
            onClose();
          }}
          className={cn(
            "block w-full px-3 py-1 text-left font-code-sm text-code-sm transition-colors hover:bg-surface-container-highest",
            it.danger ? "text-error" : "text-on-surface",
          )}
        >
          {it.label}
        </button>
      ))}
    </div>
  );
}

// ============================================================================
// ActionModal — prompt (with input) or confirm (no input) dialog. `busy` and
// `error` are parent-controlled so the parent owns the async command call.
// ============================================================================

export function ActionModal({
  title,
  message,
  inputLabel,
  initialValue,
  confirmLabel,
  danger,
  busy,
  error,
  onConfirm,
  onClose,
}: {
  title: string;
  message?: string;
  inputLabel?: string;
  initialValue?: string;
  confirmLabel: string;
  danger?: boolean;
  busy?: boolean;
  error?: string | null;
  onConfirm: (value: string) => void;
  onClose: () => void;
}) {
  const [value, setValue] = useState(initialValue ?? "");
  const hasInput = inputLabel != null;
  const canConfirm = !busy && (!hasInput || value.trim().length > 0);
  const submit = () => {
    if (!canConfirm) return;
    onConfirm(hasInput ? value.trim() : "");
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      role="dialog"
      aria-modal="true"
      aria-label={title}
      onClick={onClose}
    >
      <div
        className="w-full max-w-sm rounded border border-outline-variant bg-surface-container p-4 shadow-lg"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="font-headline-md text-headline-md text-on-surface">
          {title}
        </h2>
        {message && (
          <p className="mt-1 font-code-sm text-code-sm text-on-surface-variant">
            {message}
          </p>
        )}
        {hasInput && (
          <label className="mt-3 block">
            <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
              {inputLabel}
            </span>
            <input
              type="text"
              value={value}
              onChange={(e) => setValue(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") submit();
              }}
              autoFocus
              className={terminalInputClass}
            />
          </label>
        )}
        {error && (
          <p className="mt-2 font-code-sm text-code-sm text-error">{error}</p>
        )}
        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="rounded border border-outline-variant bg-transparent px-3 py-1 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:bg-surface-container-high hover:text-on-surface"
          >
            Cancel
          </button>
          <button
            type="button"
            disabled={!canConfirm}
            onClick={submit}
            className={cn(
              "rounded border px-3 py-1 font-code-sm text-code-sm transition-colors disabled:opacity-50",
              danger
                ? "border-error/50 bg-error/10 text-error hover:bg-error/20"
                : "border-primary bg-primary text-on-primary hover:bg-primary-fixed",
            )}
          >
            {busy ? "Working…" : confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
