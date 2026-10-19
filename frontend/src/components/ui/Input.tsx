import { forwardRef, type InputHTMLAttributes } from "react";
import { cn } from "../../lib/cn";

export const Input = forwardRef<HTMLInputElement, InputHTMLAttributes<HTMLInputElement>>(
  ({ className, ...props }, ref) => (
    <input
      ref={ref}
      className={cn(
        "w-full px-2.5 py-1.5 font-body-md text-body-md rounded border border-outline-variant bg-surface-container text-on-surface placeholder:text-subtle focus:outline-none focus:border-primary",
        className,
      )}
      {...props}
    />
  ),
);
Input.displayName = "Input";
