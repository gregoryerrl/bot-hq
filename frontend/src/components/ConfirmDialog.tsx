import { type ReactNode } from "react";
import { useFocusTrap } from "../hooks/useFocusTrap";
import { useEscapeKey } from "../hooks/useEscapeKey";
import { Button } from "./ui/Button";
import { cn } from "../lib/cn";

interface ConfirmDialogProps {
  open: boolean;
  title: string;
  /** Body copy — string or rich content. */
  message: ReactNode;
  confirmLabel?: string;
  cancelLabel?: string;
  /** `danger` for destructive/irreversible actions, `primary` otherwise. */
  confirmVariant?: "primary" | "danger";
  onConfirm: () => void;
  onCancel: () => void;
}

/**
 * Thin reusable confirmation modal. Replaces the native `window.confirm`
 * (which clashes with the Industrial Terminal styling and can't carry a
 * danger affordance). It only asks — the caller owns the action fired in
 * `onConfirm`. Backdrop click and Escape both cancel; focus is trapped while
 * open and restored to the trigger on close. Modeled on `MaintainCLModal`.
 */
export function ConfirmDialog({
  open,
  title,
  message,
  confirmLabel = "Confirm",
  cancelLabel = "Cancel",
  confirmVariant = "primary",
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const trapRef = useFocusTrap<HTMLDivElement>(open);
  useEscapeKey(onCancel, open);

  if (!open) return null;

  return (
    <>
      <div
        className="fixed inset-0 z-40 bg-black/60"
        onClick={onCancel}
        aria-hidden
      />
      <div
        ref={trapRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        className={cn(
          "fixed left-1/2 top-1/2 z-50 w-[min(440px,90vw)] -translate-x-1/2 -translate-y-1/2",
          "rounded-lg border border-outline-variant bg-surface-container p-5 shadow-2xl focus:outline-none",
        )}
      >
        <h2 className="mb-3 font-headline-md text-headline-md text-on-surface">
          {title}
        </h2>
        <div className="mb-5 font-code-sm text-code-sm text-on-surface-variant">
          {message}
        </div>
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={onCancel}>
            {cancelLabel}
          </Button>
          <Button variant={confirmVariant} onClick={onConfirm}>
            {confirmLabel}
          </Button>
        </div>
      </div>
    </>
  );
}
