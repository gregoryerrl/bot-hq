import { forwardRef, type ButtonHTMLAttributes } from "react";
import { cn } from "../../lib/cn";

type Variant = "primary" | "secondary" | "ghost" | "danger" | "link";
type Size = "sm" | "md" | "icon";
type Tone = "inherit" | "success" | "warning" | "error" | "tertiary";

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  size?: Size;
  /**
   * The label's colour, in place of the variant's own: a status colour on an
   * outline (`success` on "Restart agents"), or `inherit` for a link inside a
   * line that already has its colour (an error line's "dismiss").
   *
   * A prop, not a `className`: `cn` is plain clsx, so two text colours both
   * land and the stylesheet's order decides — which follows the class NAMES,
   * so `text-error` silently loses to the variant's `text-on-surface` while
   * `text-success` happens to win (found migrating S2b's 72 buttons).
   */
  tone?: Tone;
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
  primary: "border border-primary bg-primary hover:border-primary-fixed hover:bg-primary-fixed",
  secondary: "border border-outline-variant bg-transparent hover:bg-surface-container-high",
  ghost: "border border-transparent bg-transparent hover:bg-surface-container-high",
  danger: "border border-error-container bg-error-container hover:border-error-container/80 hover:bg-error-container/80",
  // Dotted until hovered: a link may take any tone (`inherit` too), so its
  // underline cannot be a fixed colour at partial strength.
  link: "underline decoration-dotted underline-offset-2 hover:decoration-solid",
};

/** Each variant's own label colour, replaced whole by a `tone`. */
const variantText: Record<Variant, string> = {
  primary: "text-on-primary",
  secondary: "text-on-surface",
  ghost: "text-on-surface",
  danger: "text-on-error-container",
  link: "text-primary",
};

const toneText: Record<Tone, string> = {
  inherit: "text-inherit",
  success: "text-success",
  warning: "text-warning",
  error: "text-error",
  tertiary: "text-tertiary",
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
  ({ className, variant = "secondary", size = "md", tone, type = "button", ...props }, ref) => (
    <button
      ref={ref}
      type={type}
      className={cn(
        "transition-colors disabled:cursor-not-allowed disabled:opacity-50 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-primary",
        variant === "link"
          ? "inline rounded-sm"
          : cn("inline-flex items-center justify-center rounded", sizeClasses[size]),
        variantClasses[variant],
        tone ? toneText[tone] : variantText[variant],
        className,
      )}
      {...props}
    />
  ),
);
Button.displayName = "Button";
