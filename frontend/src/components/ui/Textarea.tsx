import { forwardRef, type TextareaHTMLAttributes } from "react";
import { cn } from "../../lib/cn";

export const Textarea = forwardRef<
  HTMLTextAreaElement,
  TextareaHTMLAttributes<HTMLTextAreaElement>
>(({ className, ...props }, ref) => (
  <textarea
    ref={ref}
    className={cn(
      "w-full px-2.5 py-1.5 font-body-md text-body-md rounded border border-outline-variant bg-surface-container text-on-surface placeholder:text-subtle focus:outline-none focus:border-primary resize-y",
      className,
    )}
    {...props}
  />
));
Textarea.displayName = "Textarea";
