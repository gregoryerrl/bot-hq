import { forwardRef, type TextareaHTMLAttributes } from "react";
import { cn } from "../../lib/cn";

export const Textarea = forwardRef<
  HTMLTextAreaElement,
  TextareaHTMLAttributes<HTMLTextAreaElement>
>(({ className, ...props }, ref) => (
  <textarea
    ref={ref}
    className={cn(
      "w-full px-2.5 py-1.5 text-sm rounded border border-neutral-700 bg-neutral-900 text-neutral-100 placeholder:text-neutral-500 focus:outline-none focus:border-blue-500/60 resize-y",
      className,
    )}
    {...props}
  />
));
Textarea.displayName = "Textarea";
