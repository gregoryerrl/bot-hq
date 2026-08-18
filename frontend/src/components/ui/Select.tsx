import { forwardRef, type SelectHTMLAttributes } from "react";
import { cn } from "../../lib/cn";

/**
 * The house `<select>` (round 10). Three panels declared their own
 * `selectClass` and had already drifted (ClaudeConfig: `py-1`, no focus ring;
 * Models/Roles: `py-1.5` + `focus:ring-1`); the majority shape is the one
 * here. Sites with a deliberately different size (the Dashboard's dialog, the
 * SessionView phase select, ViolationsPanel's filters) still spell their own —
 * folding those in is a visual decision for a session with the app open.
 */
export const selectClass =
  "w-full rounded border border-outline-variant bg-surface-container-lowest px-2 py-1.5 font-code-sm text-code-sm text-on-surface focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary";

export const Select = forwardRef<HTMLSelectElement, SelectHTMLAttributes<HTMLSelectElement>>(
  ({ className, ...props }, ref) => (
    <select ref={ref} className={cn(selectClass, className)} {...props} />
  ),
);
Select.displayName = "Select";
