import React from "react";
import { cn } from "../lib/cn";

/**
 * Underline-style subtab pill shared by the Settings and Context Library
 * pages. The pill row doubles as the page/section header — panels under it
 * must NOT repeat the label as a heading. Optional `badge` renders a count
 * chip (hidden at 0) — e.g. pending questions on a session tab.
 */
export function SubTabButton({
  active,
  onClick,
  badge = 0,
  children,
}: {
  active: boolean;
  onClick: () => void;
  badge?: number;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "flex items-center gap-1.5 border-b-2 px-3 py-2.5 font-code-sm text-code-sm transition-colors",
        active
          ? "border-primary text-primary"
          : "border-transparent text-on-surface-variant hover:text-on-surface",
      )}
    >
      {children}
      {badge > 0 && (
        <span className="rounded-full bg-primary px-1.5 text-[10px] font-semibold leading-4 text-on-primary">
          {badge}
        </span>
      )}
    </button>
  );
}
