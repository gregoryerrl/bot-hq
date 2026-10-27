import type { SVGProps, ReactNode } from "react";
import { cn } from "../lib/cn";

/**
 * What the base owns, so no call site can drift from the set: the 24-unit
 * canvas, the outline look, and the sizing mechanism (a class — never the
 * `width`/`height` attributes).
 */
type Owned =
  | "children"
  | "viewBox"
  | "width"
  | "height"
  | "fill"
  | "stroke"
  | "strokeWidth"
  | "strokeLinecap"
  | "strokeLinejoin"
  | "aria-hidden";

/** Every other SVG attribute passes through — `className`, `style`, handlers. */
export type IconProps = Omit<SVGProps<SVGSVGElement>, Owned>;

/**
 * A caller's own size: a `size-*`, `w-*` or `h-*` utility, after any variant
 * prefix (`md:`, `group-hover:`) or the `!` modifier.
 */
const CALLER_SIZED = /(?:^|\s)(?:\S+:)?!?(?:size|w|h)-\S/;

/**
 * **One icon set** (the 2026-10-19 visual pass, slice S1d): every icon is drawn
 * on this one base — a 24-unit outline, `fill="none"`, `stroke="currentColor"`
 * at 1.75 with round caps and joins, so it inherits the surrounding text colour
 * — and is `aria-hidden="true"`: an icon is never an accessible name, so an
 * icon-only button carries its own `aria-label`. Use these instead of emoji and
 * dingbat glyphs (✕ ▸ ⚠ ✓ ⏸ …), which render filled, in whatever the font
 * draws, and clash with the Industrial Terminal styling.
 *
 * Sized by CLASS: `size-4` (16 px) unless the caller's `className` sizes the
 * icon itself. The default is DROPPED then, not out-ordered: `cn` is plain
 * `clsx` (no tailwind-merge), so `size-4 size-3` would resolve by stylesheet
 * order, where `.size-4` sorts after `.size-3` and the caller's size would lose.
 *
 * Preflight makes every `svg` a block. An icon that stands where a glyph sat
 * inside a run of text therefore takes `inline-block align-middle`; one beside
 * text in a flex row needs neither.
 *
 * Until S1d there were two bases — stroke 1.75 at a 16 px attribute size for
 * the app chrome, stroke 2 at a `size-3.5` class for the Context Library
 * family — kept apart so that merging the two icon modules changed no pixels.
 * The near-twins that merge left (`MemoryIcon`/`FileIcon`,
 * `RescanIcon`/`RefreshIcon`) are still two drawings each.
 */
function Svg({ className, children, ...props }: IconProps & { children: ReactNode }) {
  return (
    <svg
      {...props}
      className={cn(!CALLER_SIZED.test(className ?? "") && "size-4", className)}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.75}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {children}
    </svg>
  );
}

/** Saved-model favorite; callers may fill the selected heart with currentColor. */
export function HeartIcon(props: IconProps) {
  return <Svg {...props}><path d="M20.8 4.6a5.5 5.5 0 0 0-7.8 0L12 5.7l-1.1-1.1a5.5 5.5 0 0 0-7.8 7.8L12 21l8.8-8.6a5.5 5.5 0 0 0 0-7.8Z" /></Svg>;
}

/** Notification bell — the pending-tray indicator. */
export function BellIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9" />
      <path d="M13.73 21a2 2 0 0 1-3.46 0" />
    </Svg>
  );
}

/** Settings gear — replaces ⚙ / ⚙️. */
export function GearIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
    </Svg>
  );
}

/** Target / overview — replaces ◉. */
export function OverviewIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <circle cx="12" cy="12" r="9" />
      <circle cx="12" cy="12" r="3.5" />
    </Svg>
  );
}

/** Sparkle / skills — replaces ✦. */
export function SkillsIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M12 3l1.9 6.1L20 11l-6.1 1.9L12 19l-1.9-6.1L4 11l6.1-1.9z" />
    </Svg>
  );
}

/** Overlapping squares / plugins — replaces ⧉. */
export function PluginsIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <rect x="3.5" y="3.5" width="12" height="12" rx="1.5" />
      <path d="M8.5 20.5h10a2 2 0 0 0 2-2v-10" />
    </Svg>
  );
}

/** Exchange arrows / MCP — replaces ⇄. */
export function McpIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M4 9h13" />
      <path d="M13 5l4 4-4 4" />
      <path d="M20 15H7" />
      <path d="M11 11l-4 4 4 4" />
    </Svg>
  );
}

/** Document / memory & instructions — replaces ❏. */
export function MemoryIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z" />
      <path d="M14 3v6h6" />
    </Svg>
  );
}

/** Key / permissions — replaces ⚿. */
export function PermissionsIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <circle cx="8" cy="15" r="4" />
      <path d="M10.85 12.15 20 3" />
      <path d="M16 7l3 3" />
      <path d="M18.5 4.5l2 2" />
    </Svg>
  );
}

/** Circular refresh / rescan — replaces ↻. */
export function RescanIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M21 12a9 9 0 1 1-2.64-6.36" />
      <path d="M21 4v5h-5" />
    </Svg>
  );
}

/** Trash can — discarding a tray card without answering it. */
export function TrashIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M3 6h18" />
      <path d="M8 6V4a1 1 0 0 1 1-1h6a1 1 0 0 1 1 1v2" />
      <path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6" />
      <path d="M10 11v6" />
      <path d="M14 11v6" />
    </Svg>
  );
}

/** Alert triangle / warning — replaces ⚠. */
export function WarnIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M10.3 3.86 1.82 18a2 2 0 0 0 1.7 3h16.96a2 2 0 0 0 1.7-3L13.7 3.86a2 2 0 0 0-3.4 0z" />
      <path d="M12 9v4" />
      <path d="M12 17h.01" />
    </Svg>
  );
}

// ============================================================================
// The Context Library family (moved here from `app/contextLibraryShared.tsx`,
// round 9). On its own stroke-2, `size-3.5` base until S1d.
// ============================================================================

/** Plus — register / add. */
export function PlusIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <line x1="12" y1="5" x2="12" y2="19" />
      <line x1="5" y1="12" x2="19" y2="12" />
    </Svg>
  );
}

/** Refresh / rescan (the Context Library's rescan button; `animate-spin`
 *  while running). Near-twin of `RescanIcon` — different arc geometry. */
export function RefreshIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <polyline points="23 4 23 10 17 10" />
      <path d="M20.49 15a9 9 0 11-2.12-9.36L23 10" />
    </Svg>
  );
}

/** Document with a folded corner — a CL file / workspace file. Near-twin of
 *  `MemoryIcon` (1-unit y offset). */
export function FileIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" />
      <polyline points="14 2 14 8 20 8" />
    </Svg>
  );
}

/** Eye — agent-visible. */
export function EyeIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M1 12s4-7 11-7 11 7 11 7-4 7-11 7-11-7-11-7z" />
      <circle cx="12" cy="12" r="3" />
    </Svg>
  );
}

/** Eye, slashed — hidden from agents. */
export function EyeOffIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M17.94 17.94A10.07 10.07 0 0112 20c-7 0-11-8-11-8a18.45 18.45 0 015.06-5.94" />
      <path d="M9.9 4.24A9.12 9.12 0 0112 4c7 0 11 8 11 8a18.5 18.5 0 01-2.16 3.19" />
      <path d="M14.12 14.12a3 3 0 11-4.24-4.24" />
      <line x1="1" y1="1" x2="23" y2="23" />
    </Svg>
  );
}

/** Folder. */
export function FolderIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M3 7a2 2 0 012-2h4l2 2h8a2 2 0 012 2v8a2 2 0 01-2 2H5a2 2 0 01-2-2z" />
    </Svg>
  );
}

/** Close × — the dismiss ACTION; replaces ✕. (`CrossIcon` is the status mark.) */
export function CloseIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <line x1="18" y1="6" x2="6" y2="18" />
      <line x1="6" y1="6" x2="18" y2="18" />
    </Svg>
  );
}

/** Save (floppy). */
export function SaveIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M19 21H5a2 2 0 01-2-2V5a2 2 0 012-2h11l5 5v11a2 2 0 01-2 2z" />
      <polyline points="17 21 17 13 7 13 7 21" />
      <polyline points="7 3 7 8 15 8" />
    </Svg>
  );
}

/** "Collapse all" — two chevrons meeting at the center (top points down,
 *  bottom points up), the VS-Code tree-toolbar fold glyph. */
export function CollapseAllIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <polyline points="7 8 12 12 17 8" />
      <polyline points="7 16 12 12 17 16" />
    </Svg>
  );
}

// ============================================================================
// Glyph replacements (S1d) — each one retires a text glyph that stood in for
// an icon.
// ============================================================================

/** Chevron, pointing right — a closed disclosure; replaces ▸ / ▶. Turn it
 *  with `rotate-90` for an animated open state, or swap in `ChevronDownIcon`. */
export function ChevronRightIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M8.5 5l7 7-7 7" />
    </Svg>
  );
}

/** Chevron, pointing down — an open disclosure; replaces ▾. */
export function ChevronDownIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M5 8.5l7 7 7-7" />
    </Svg>
  );
}

/** Check mark — done, passed, saved, staged; replaces ✓. */
export function CheckIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M5 12.5l4.5 4.5L19 7.5" />
    </Svg>
  );
}

/** Cross mark — failed; replaces ✗. Drawn to `CheckIcon`'s size, so the pair
 *  reads as one; the larger `CloseIcon` is the dismiss action. */
export function CrossIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M7 7l10 10" />
      <path d="M17 7 7 17" />
    </Svg>
  );
}

/** Two bars — paused / halted; replaces ⏸. */
export function PauseIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M9 5v14" />
      <path d="M15 5v14" />
    </Svg>
  );
}

/** Clock face — scheduled; replaces ⏱. */
export function ClockIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 7v5l3 2" />
    </Svg>
  );
}

/** Git branch — a session's worktree branch; replaces ⎇. */
export function BranchIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M6 3v12" />
      <circle cx="18" cy="6" r="3" />
      <circle cx="6" cy="18" r="3" />
      <path d="M18 9a9 9 0 0 1-9 9" />
    </Svg>
  );
}

/** Pencil — edit / rename; replaces ✎. */
export function EditIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4z" />
    </Svg>
  );
}

/** Arrow, pointing left — back. */
export function ArrowLeftIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M19 12H5" />
      <path d="M11 18l-6-6 6-6" />
    </Svg>
  );
}

/** Arrow, pointing right — forward / open. */
export function ArrowRightIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M5 12h14" />
      <path d="M13 6l6 6-6 6" />
    </Svg>
  );
}

/** Skip forward — a passed turn; replaces ⏭. */
export function SkipIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M5 5l10 7-10 7z" />
      <path d="M19 5v14" />
    </Svg>
  );
}

/** A filled dot — an unsaved / live marker; replaces •. The one filled shape
 *  in the set: a dot drawn in outline would read as a ring. */
export function DotIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <circle cx="12" cy="12" r="4" fill="currentColor" />
    </Svg>
  );
}
