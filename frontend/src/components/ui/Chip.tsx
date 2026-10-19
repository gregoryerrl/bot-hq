import type { HTMLAttributes, ReactNode } from "react";
import { cn } from "../../lib/cn";
import { CheckIcon } from "../icons";

export type ChipTone = "neutral" | "primary" | "secondary" | "tertiary" | "success" | "warning" | "error";
type ChipSize = "sm" | "md";

/**
 * **The one chip** (the 2026-10-19 visual pass, S2d). DESIGN.md's status chip:
 * a 15% tint of the tone's colour with the colour at full strength for the
 * text, a 40% outline of the same colour, the house 2px radius and the
 * `label-caps` type. The tones are the theme's: the role colours (`primary`,
 * `secondary`, `tertiary` — the user's blue) and the status colours.
 * `neutral` is a quiet grey chip for a count or a kind.
 *
 * The classes are spelled out whole, never assembled from the tone's name:
 * Tailwind only generates a class it can read in the source.
 */
const toneClasses: Record<ChipTone, string> = {
  neutral: "border-outline-variant bg-surface-container-high text-on-surface-variant",
  primary: "border-primary/40 bg-primary/15 text-primary",
  secondary: "border-secondary/40 bg-secondary/15 text-secondary",
  tertiary: "border-tertiary/40 bg-tertiary/15 text-tertiary",
  success: "border-success/40 bg-success/15 text-success",
  warning: "border-warning/40 bg-warning/15 text-warning",
  error: "border-error/40 bg-error/15 text-error",
};

const sizeClasses: Record<ChipSize, string> = {
  sm: "px-1 py-px",
  md: "px-1.5 py-0.5",
};

/** A tone's tint classes alone, for a surface that is not a chip but takes
 *  the same tint (a status row, a toggle's active state). */
export function chipTone(tone: ChipTone): string {
  return toneClasses[tone];
}

/**
 * A chip is one unbroken label by default. `wrap` lets a long status label
 * ("Needs you · halted — your move") break onto a second line inside a narrow
 * tile instead of being clipped by an `overflow-hidden` parent (EYES, S2d).
 */
export function Chip({
  tone = "neutral",
  size = "md",
  wrap = false,
  className,
  ...props
}: HTMLAttributes<HTMLSpanElement> & { tone?: ChipTone; size?: ChipSize; wrap?: boolean }) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded border font-label-caps text-label-caps",
        wrap ? "min-w-0 max-w-full whitespace-normal break-words" : "shrink-0 whitespace-nowrap",
        toneClasses[tone],
        sizeClasses[size],
        className,
      )}
      {...props}
    />
  );
}

/**
 * An editor's save state, beside its Save button: "Unsaved" while there are
 * edits, "Saved" with a check once they are written, nothing otherwise. One
 * component for what four editors spelled by hand.
 */
export function SaveStateChip({ dirty, saved }: { dirty: boolean; saved: boolean }): ReactNode {
  if (dirty) return <Chip tone="warning">Unsaved</Chip>;
  if (saved) {
    return (
      <Chip tone="success">
        Saved
        <CheckIcon className="size-3" />
      </Chip>
    );
  }
  return null;
}
