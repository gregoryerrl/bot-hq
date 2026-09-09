import { forwardRef, type ButtonHTMLAttributes } from "react";
import { cn } from "../../lib/cn";

type Variant = "primary" | "secondary" | "ghost" | "danger";
type Size = "sm" | "md" | "lg";

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  size?: Size;
}

// Migrated to Industrial Terminal tokens. `primary` is the first-author
// orange (`bg-primary` → `#ffb68b`), `secondary` is the muted surface tier,
// `ghost` is transparent on a hovered surface, `danger` uses the error
// container token for destructive intent.
const variantClasses: Record<Variant, string> = {
  primary: "bg-primary hover:bg-primary-fixed-dim text-on-primary",
  secondary: "bg-surface-container hover:bg-surface-container-high text-on-surface",
  ghost: "bg-transparent hover:bg-surface-container/60 text-on-surface",
  danger: "bg-error-container hover:bg-error-container/80 text-on-error-container",
};

// One radius for every size — the house `rounded` (2px, the Industrial
// Terminal's squared-off input shape); md/lg used to be `rounded-md` (6px),
// so a Save button and the input beside it never matched (round 11).
const sizeClasses: Record<Size, string> = {
  sm: "px-2 py-1 text-xs rounded",
  md: "px-3 py-1.5 text-sm rounded",
  lg: "px-4 py-2 text-base rounded",
};

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant = "secondary", size = "md", ...props }, ref) => (
    <button
      ref={ref}
      className={cn(
        "inline-flex items-center justify-center gap-2 font-medium transition-colors disabled:opacity-50 disabled:cursor-not-allowed focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-primary",
        variantClasses[variant],
        sizeClasses[size],
        className,
      )}
      {...props}
    />
  ),
);
Button.displayName = "Button";
