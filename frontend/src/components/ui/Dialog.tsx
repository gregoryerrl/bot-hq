import { useId, type ReactNode } from "react";
import { cn } from "../../lib/cn";
import { useEscapeKey } from "../../hooks/useEscapeKey";
import { useFocusTrap } from "../../hooks/useFocusTrap";
import { CloseIcon } from "../icons";
import { Button } from "./Button";

/**
 * The house WIDE dialog frame — New session and Add/Edit model. FIXED (user
 * 2026-08-25): the size is the viewport-clamped constant, never the content.
 * Content that grows — participants added to the roster, the model dialog's
 * setup commands appearing on the first typed character of a config dir —
 * scrolls inside the frame instead of moving its edges, and removing it does
 * not shrink the frame. The min() clamp keeps both edges on-screen for
 * short/narrow windows (a fixed+translated box cannot be scrolled back into
 * view by the page).
 *
 * It is a flex column: the header and the footer take `shrink-0`, the body
 * `min-h-0 flex-1` plus its own scroller, so the actions stay in view at any
 * window size. Render the scrim as the frame's SIBLING, never its parent: a
 * drag-select that starts inside the frame and is released over a parent
 * overlay counts as a click on the overlay, which would close the dialog.
 */
export const wideDialogClass = cn(
  "fixed left-1/2 top-1/2 z-50 flex h-[min(760px,90vh)] w-[min(1100px,92vw)] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden",
  "rounded-lg border border-outline-variant bg-surface-container p-5 shadow-2xl focus:outline-none",
);

/** The one scrim behind a dialog (the visual pass, S2f): the theme's `scrim`
 *  at 60%, under the frame (`z-40` / `z-50`). */
export const scrimClass = "fixed inset-0 z-40 bg-scrim/60";

type DialogSize = "sm" | "md" | "lg";

const frameSize: Record<DialogSize, string> = {
  sm: "max-h-[85vh] w-[min(440px,90vw)]",
  md: "max-h-[85vh] w-[min(640px,92vw)]",
  // The wide frame's fixed, viewport-clamped size (see `wideDialogClass`).
  lg: "h-[min(760px,90vh)] w-[min(1100px,92vw)]",
};

/**
 * **The one dialog** (the visual pass, S2f): a scrim, and beside it — never
 * around it, for the drag-select reason above — a frame with a title row (the
 * title, then a close button), a body that scrolls on its own, and an optional
 * footer for the actions, which therefore never leave the window. Escape, the
 * scrim and the close button all call `onClose`; a caller with unsaved edits
 * passes its guarded close. Focus moves into the frame on open, is trapped
 * there, and returns to the trigger on close (`useFocusTrap`).
 */
export function Dialog({
  open,
  onClose,
  title,
  size = "sm",
  footer,
  closeLabel = "Close",
  className,
  bodyClassName,
  children,
}: {
  open: boolean;
  onClose: () => void;
  title: ReactNode;
  size?: DialogSize;
  footer?: ReactNode;
  /** The close button's accessible name. */
  closeLabel?: string;
  className?: string;
  bodyClassName?: string;
  children: ReactNode;
}) {
  const trapRef = useFocusTrap<HTMLDivElement>(open);
  useEscapeKey(onClose, open);
  const titleId = useId();
  if (!open) return null;
  return (
    <>
      <div className={scrimClass} onClick={onClose} aria-hidden data-testid="dialog-scrim" />
      <div
        ref={trapRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className={cn(
          "fixed left-1/2 top-1/2 z-50 flex -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden",
          "rounded-lg border border-outline-variant bg-surface-container shadow-2xl focus:outline-none",
          frameSize[size],
          className,
        )}
      >
        <div className="flex shrink-0 items-start justify-between gap-3 px-panel-padding pt-panel-padding">
          <h2 id={titleId} className="min-w-0 break-words font-headline-md text-headline-md text-on-surface">
            {title}
          </h2>
          <Button variant="ghost" size="icon" aria-label={closeLabel} onClick={onClose} className="-mr-1 -mt-0.5">
            <CloseIcon />
          </Button>
        </div>
        <div
          className={cn(
            "min-h-0 flex-1 overflow-y-auto overflow-x-hidden px-panel-padding py-3",
            !footer && "pb-panel-padding",
            bodyClassName,
          )}
        >
          {children}
        </div>
        {footer && (
          <div className="flex shrink-0 flex-wrap items-center justify-end gap-2 border-t border-outline-variant px-panel-padding py-3">
            {footer}
          </div>
        )}
      </div>
    </>
  );
}
