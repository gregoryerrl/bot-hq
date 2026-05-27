import { forwardRef, type ButtonHTMLAttributes } from "react";
import { cn } from "../../lib/cn";

type Variant = "primary" | "secondary" | "ghost" | "danger";
type Size = "sm" | "md" | "lg";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  size?: Size;
}

// Migrated to Industrial Terminal tokens. `primary` is the role-Brian
// orange (`bg-primary` → `#ffb68b`), `secondary` is the muted surface tier,
// `ghost` is transparent on a hovered surface, `danger` keeps red for
// destructive intent (no design-system red yet).
const variantClasses: Record<Variant, string> = {
  primary: "bg-primary hover:bg-primary-fixed-dim text-on-primary",
  secondary: "bg-surface-container hover:bg-surface-container-high text-on-surface",
  ghost: "bg-transparent hover:bg-surface-container/60 text-on-surface",
  danger: "bg-red-600 hover:bg-red-500 text-white",
};

const sizeClasses: Record<Size, string> = {
  sm: "px-2 py-1 text-xs rounded",
  md: "px-3 py-1.5 text-sm rounded-md",
  lg: "px-4 py-2 text-base rounded-md",
};

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant = "secondary", size = "md", ...props }, ref) => (
    <button
      ref={ref}
      className={cn(
        "inline-flex items-center justify-center font-medium transition-colors disabled:opacity-50 disabled:cursor-not-allowed",
        variantClasses[variant],
        sizeClasses[size],
        className,
      )}
      {...props}
    />
  ),
);
Button.displayName = "Button";
