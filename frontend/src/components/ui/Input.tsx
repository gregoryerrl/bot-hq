import { forwardRef, type InputHTMLAttributes } from "react";
import { cn } from "../../lib/cn";

export const Input = forwardRef<HTMLInputElement, InputHTMLAttributes<HTMLInputElement>>(
  ({ className, ...props }, ref) => (
    <input
      ref={ref}
      className={cn(
        "w-full px-2.5 py-1.5 text-sm rounded border border-neutral-700 bg-neutral-900 text-neutral-100 placeholder:text-neutral-500 focus:outline-none focus:border-blue-500/60",
        className,
      )}
      {...props}
    />
  ),
);
Input.displayName = "Input";
