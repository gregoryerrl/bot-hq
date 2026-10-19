import { forwardRef, type ButtonHTMLAttributes } from "react";
import { cn } from "../../lib/cn";

type Variant = "primary" | "secondary" | "ghost" | "danger" | "link";
type Size = "sm" | "md" | "icon";

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  size?: Size;
}

/**
 * The one button (the 2026-10-19 visual pass, S2). Every variant carries a
 * 1px border, transparent where the variant has none, so a primary and a
 * secondary button side by side are the same size.
 *
 * - `primary` is the executing role's orange, filled; it lightens on hover
 *   (`primary-fixed`), since `primary-fixed-dim` is the same hex as `primary`.
 * - `secondary` (the default) is an outline: the hairline-family
 *   `outline-variant` border on a transparent fill — the Industrial Terminal's
 *   low-contrast outline, and the look most hand-rolled buttons already had.
 * - `ghost` has no outline until hovered — toolbar and icon buttons.
 * - `danger` is the error container, for destructive intent.
 * - `link` is an action that reads as a link: no box, no size of its own, so it
 *   sits in a run of text at the text's size.
 */
const variantClasses: Record<Variant, string> = {
  primary: "border border-primary bg-primary text-on-primary hover:border-primary-fixed hover:bg-primary-fixed",
  secondary: "border border-outline-variant bg-transparent text-on-surface hover:bg-surface-container-high",
  ghost: "border border-transparent bg-transparent text-on-surface hover:bg-surface-container-high",
  danger:
    "border border-error-container bg-error-container text-on-error-container hover:border-error-container/80 hover:bg-error-container/80",
  link: "text-primary underline decoration-primary/40 underline-offset-2 hover:decoration-primary",
};

/**
 * Labels are mono (`code-sm`): a button is a label, and the Industrial
 * Terminal sets labels in JetBrains Mono. The sizes differ in padding only.
 * One radius for every size — the house `rounded` (2px). `icon` is a 28px
 * square for an icon-only button, which must carry its own `aria-label`.
 */
const sizeClasses: Record<Size, string> = {
  sm: "gap-1.5 px-2 py-1 font-code-sm text-code-sm",
  md: "gap-2 px-3 py-1.5 font-code-sm text-code-sm",
  icon: "size-7 shrink-0",
};

/**
 * `type` defaults to `"button"`: a typeless button inside a `<form>` (the
 * composer is one) submits it — the browser's default, never what a click on
 * Copy or Cancel means. Pass `type="submit"` for the one that should.
 */
export const Button = forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant = "secondary", size = "md", type = "button", ...props }, ref) => (
    <button
      ref={ref}
      type={type}
      className={cn(
        "transition-colors disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-primary",
        variant === "link"
          ? "inline rounded-sm"
          : cn("inline-flex items-center justify-center rounded", sizeClasses[size]),
        variantClasses[variant],
        className,
      )}
      {...props}
    />
  ),
);
Button.displayName = "Button";
