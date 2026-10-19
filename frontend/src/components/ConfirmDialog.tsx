import { type ReactNode } from "react";
import { Button } from "./ui/Button";
import { Dialog } from "./ui/Dialog";

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
 * `onConfirm`. Built on the one `Dialog` (the visual pass, S2f): the scrim,
 * Escape and the close button all cancel; focus is trapped while open and
 * restored to the trigger on close. The body scrolls on its own (a plugin's
 * install consent can run long), so the buttons never leave the window. The
 * message is prose, so it is set in Inter (DESIGN.md amendment 1).
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
  return (
    <Dialog
      open={open}
      onClose={onCancel}
      title={title}
      bodyClassName="font-body-md text-body-md text-on-surface"
      footer={
        <>
          <Button variant="ghost" onClick={onCancel}>
            {cancelLabel}
          </Button>
          <Button variant={confirmVariant} onClick={onConfirm}>
            {confirmLabel}
          </Button>
        </>
      }
    >
      {message}
    </Dialog>
  );
}
