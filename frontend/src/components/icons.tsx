import type { SVGProps, ReactNode } from "react";
import { cn } from "../lib/cn";

export type IconProps = SVGProps<SVGSVGElement> & { size?: number };

/**
 * Shared outline-icon base — `fill="none"`, `stroke="currentColor"`, round
 * caps/joins — so every icon reads as an outline that inherits the surrounding
 * text color. Use these instead of emoji/dingbat glyphs (which render filled
 * and clash with the Industrial Terminal styling).
 */
function Svg({
  size = 16,
  children,
  ...props
}: IconProps & { children: ReactNode }) {
  return (
    <svg
      aria-hidden
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="none"
      stroke="currentColor"
      strokeWidth={1.75}
      strokeLinecap="round"
      strokeLinejoin="round"
      {...props}
    >
      {children}
    </svg>
  );
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
// Class-sized family (moved here from `app/contextLibraryShared.tsx`, round 9)
// ============================================================================

/**
 * The second base: `strokeWidth={2}` and sized by a Tailwind `size-*` CLASS
 * (default `size-3.5` = 14 px) instead of width/height attributes. It exists
 * so consolidating the icon modules changed no pixels — every Context Library
 * caller was written against these defaults. Unifying the two bases (stroke
 * 1.75 vs 2, attribute-sized 16 px vs class-sized 14 px) is a visual decision
 * to make with the app open, not a refactor; likewise fusing the near-twins
 * (`MemoryIcon`/`FileIcon`, `RescanIcon`/`RefreshIcon`). `cn` is plain
 * `clsx` — no tailwind-merge — so a caller's `size-*` only wins when it is
 * larger than the default (`.size-4` sorts after `.size-3.5`).
 */
function ClassSvg({
  className,
  children,
}: {
  className?: string;
  children: ReactNode;
}) {
  return (
    <svg
      className={cn("size-3.5", className)}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      {children}
    </svg>
  );
}

type ClassIconProps = { className?: string };

/** Plus — register / add. (Defaulted `size-4` until round 9, which made its
 *  one caller's `className="size-3.5"` a no-op; `size-3.5` like its siblings.) */
export function PlusIcon({ className }: ClassIconProps) {
  return (
    <ClassSvg className={className}>
      <line x1="12" y1="5" x2="12" y2="19" />
      <line x1="5" y1="12" x2="19" y2="12" />
    </ClassSvg>
  );
}

/** Refresh / rescan (the Context Library's rescan button; `animate-spin`
 *  while running). Near-twin of `RescanIcon` — different arc geometry. */
export function RefreshIcon({ className }: ClassIconProps) {
  return (
    <ClassSvg className={className}>
      <polyline points="23 4 23 10 17 10" />
      <path d="M20.49 15a9 9 0 11-2.12-9.36L23 10" />
    </ClassSvg>
  );
}

/** Document with a folded corner — a CL file / workspace file. Near-twin of
 *  `MemoryIcon` (1-unit y offset). */
export function FileIcon({ className }: ClassIconProps) {
  return (
    <ClassSvg className={className}>
      <path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" />
      <polyline points="14 2 14 8 20 8" />
    </ClassSvg>
  );
}

/** Eye — agent-visible. */
export function EyeIcon({ className }: ClassIconProps) {
  return (
    <ClassSvg className={className}>
      <path d="M1 12s4-7 11-7 11 7 11 7-4 7-11 7-11-7-11-7z" />
      <circle cx="12" cy="12" r="3" />
    </ClassSvg>
  );
}

/** Eye, slashed — hidden from agents. */
export function EyeOffIcon({ className }: ClassIconProps) {
  return (
    <ClassSvg className={className}>
      <path d="M17.94 17.94A10.07 10.07 0 0112 20c-7 0-11-8-11-8a18.45 18.45 0 015.06-5.94" />
      <path d="M9.9 4.24A9.12 9.12 0 0112 4c7 0 11 8 11 8a18.5 18.5 0 01-2.16 3.19" />
      <path d="M14.12 14.12a3 3 0 11-4.24-4.24" />
      <line x1="1" y1="1" x2="23" y2="23" />
    </ClassSvg>
  );
}

/** Folder. */
export function FolderIcon({ className }: ClassIconProps) {
  return (
    <ClassSvg className={className}>
      <path d="M3 7a2 2 0 012-2h4l2 2h8a2 2 0 012 2v8a2 2 0 01-2 2H5a2 2 0 01-2-2z" />
    </ClassSvg>
  );
}

/** Close ×. */
export function CloseIcon({ className }: ClassIconProps) {
  return (
    <ClassSvg className={className}>
      <line x1="18" y1="6" x2="6" y2="18" />
      <line x1="6" y1="6" x2="18" y2="18" />
    </ClassSvg>
  );
}

/** Save (floppy). */
export function SaveIcon({ className }: ClassIconProps) {
  return (
    <ClassSvg className={className}>
      <path d="M19 21H5a2 2 0 01-2-2V5a2 2 0 012-2h11l5 5v11a2 2 0 01-2 2z" />
      <polyline points="17 21 17 13 7 13 7 21" />
      <polyline points="7 3 7 8 15 8" />
    </ClassSvg>
  );
}

/** "Collapse all" — two chevrons meeting at the center (top points down,
 *  bottom points up), the VS-Code tree-toolbar fold glyph. */
export function CollapseAllIcon({ className }: ClassIconProps) {
  return (
    <ClassSvg className={className}>
      <polyline points="7 8 12 12 17 8" />
      <polyline points="7 16 12 12 17 16" />
    </ClassSvg>
  );
}
