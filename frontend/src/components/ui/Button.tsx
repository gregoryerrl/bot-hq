import { forwardRef, type ButtonHTMLAttributes } from "react";
import { cn } from "../../lib/cn";

type Variant = "primary" | "secondary" | "ghost" | "danger";
type Size = "sm" | "md" | "lg";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  size?: Size;
}

const variantClasses: Record<Variant, string> = {
  primary: "bg-blue-600 hover:bg-blue-500 text-white",
  secondary: "bg-neutral-800 hover:bg-neutral-700 text-neutral-100",
  ghost: "bg-transparent hover:bg-neutral-800/60 text-neutral-100",
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
