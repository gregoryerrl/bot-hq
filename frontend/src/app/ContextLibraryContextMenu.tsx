import { useEffect, useRef, useState } from "react";
import { cn } from "../lib/cn";
import { Button } from "../components/ui/Button";
import { Dialog } from "../components/ui/Dialog";
import { fieldClass } from "../components/ui/field";

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

  // The one Dialog (S2f), always open — the parent mounts this only while an
  // action is pending. Escape, the scrim and the close button all call
  // `onClose`. Its scrim is the frame's sibling: this modal's used to be the
  // frame's parent, so a drag-select released over it closed the modal.
  return (
    <Dialog
      open
      onClose={onClose}
      title={title}
      footer={
        <>
          <Button variant="secondary" size="sm" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant={danger ? "danger" : "primary"}
            size="sm"
            disabled={!canConfirm}
            onClick={submit}
          >
            {busy ? "Working…" : confirmLabel}
          </Button>
        </>
      }
    >
      {message && (
        <p className="font-code-sm text-code-sm text-on-surface-variant">
          {message}
        </p>
      )}
      {hasInput && (
        <label className="mt-3 block first:mt-0">
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
            className={fieldClass}
          />
        </label>
      )}
      {error && (
        <p className="mt-2 font-code-sm text-code-sm text-error">{error}</p>
      )}
    </Dialog>
  );
}
