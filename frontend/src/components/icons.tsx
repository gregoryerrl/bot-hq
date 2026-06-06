import type { SVGProps, ReactNode } from "react";

export type IconProps = SVGProps<SVGSVGElement> & { size?: number };

/**
 * Shared outline-icon base. Matches the house style of `PendingTray`'s
 * `BellIcon` — `fill="none"`, `stroke="currentColor"`, round caps/joins — so
 * every icon reads as an outline that inherits the surrounding text color.
 * Use these instead of emoji/dingbat glyphs (which render filled and clash
 * with the Industrial Terminal styling).
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

/** Wrench / HANDS agent (Brian) — replaces the 👷 avatar. */
export function WrenchIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z" />
    </Svg>
  );
}

/** Eye / EYES agent (Rain) — replaces the 💧 avatar; matches the role name. */
export function EyeIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7-10-7-10-7z" />
      <circle cx="12" cy="12" r="3" />
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
