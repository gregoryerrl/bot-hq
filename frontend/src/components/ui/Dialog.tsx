import { cn } from "../../lib/cn";

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
