import React from "react";
import { cn } from "../lib/cn";

/**
 * Underline-style subtab pill shared by the Settings, Context Library and
 * Session pages. The pill row is the page's navigation; a panel under it may
 * open with its own `<h2>` and a descriptive paragraph (the Shell's wordmark
 * is the page's one `<h1>`) — round 11 retired the older "must not repeat the
 * label" rule here, which six panels never followed.
 *
 * Rendered as `role="tab"` with `aria-selected` (round 10): the rows were
 * bare buttons whose active state was colour-only, next to `role="tabpanel"`
 * panels that named no tab. Give the row `role="tablist"` and, when the panel
 * has an id, pass `controls` so the pair is announced. (An earlier optional
 * `badge` count chip had no caller anywhere and is gone.)
 */
export function SubTabButton({
  active,
  onClick,
  controls,
  children,
}: {
  active: boolean;
  onClick: () => void;
  /** The `id` of the panel this tab shows, for `aria-controls`. */
  controls?: string;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      role="tab"
      aria-selected={active}
      aria-controls={controls}
      onClick={onClick}
      className={cn(
        "flex items-center gap-1.5 border-b-2 px-3 py-2.5 font-code-sm text-code-sm transition-colors",
        active
          ? "border-primary text-primary"
          : "border-transparent text-on-surface-variant hover:text-on-surface",
      )}
    >
      {children}
    </button>
  );
}
