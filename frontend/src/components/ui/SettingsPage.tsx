import type { ReactNode } from "react";
import { cn } from "../../lib/cn";

/**
 * **One Settings page frame** (the 2026-10-19 visual pass, S5). Every Settings
 * panel that is a page (not a two-pane workspace like Roles or Claude Config)
 * sits in this: the SCROLLER spans the whole panel, so its scrollbar is at the
 * window's edge, and the content is one centred column of one width.
 *
 * Before it, each panel made its max-width box the scroller — 4xl, 5xl or 7xl
 * by panel — so the pages jumped in width as you switched tabs, and a narrow
 * page's scrollbar floated mid-screen (Policy).
 */
export function SettingsPage({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <div className="h-full overflow-y-auto overflow-x-hidden">
      <div className={cn(SETTINGS_PAGE_COLUMN, className)}>{children}</div>
    </div>
  );
}

/** The page column's width and padding — shared with the two-pane panels so
 *  every Settings tab starts at the same left edge. */
export const SETTINGS_PAGE_COLUMN = "mx-auto max-w-6xl px-6 py-6";
