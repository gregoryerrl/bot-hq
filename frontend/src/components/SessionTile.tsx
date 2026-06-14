import { useNavigate } from "react-router-dom";
import { cn } from "../lib/cn";
import { formatRelative } from "../lib/time";
import type { SessionInfo } from "../lib/bindings";
import { SessionPhaseChip, phaseTintClasses } from "./SessionPhaseChip";
import { useHealthStore, worstHealth } from "../stores/health";
import { HealthDot } from "./HealthDot";

export interface SessionTileProps {
  session: SessionInfo;
  /** Count of items awaiting the user for this session (durable tray). The tile
   *  only INDICATES — the user answers on the session's Tray tab. */
  pendingCount?: number;
  /** Current IPAV phase (lowercase) from `get_session_phase`. Null when unknown. */
  phase?: string | null;
}

export function SessionTile({
  session,
  pendingCount = 0,
  phase = null,
}: SessionTileProps) {
  const navigate = useNavigate();
  const closed = session.closed_at !== null;
  const needsInput = pendingCount > 0;
  const tint = phaseTintClasses(phase, closed);
  // B2: session-level health dot (problem-only on the tile). Worst-of Brian+Rain.
  const health = useHealthStore((s) => s.bySession[session.id]);

  const open = () => navigate(`/sessions/${session.id}`);
  const onTileKey = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      open();
    }
  };

  return (
    <div
      role="link"
      tabIndex={0}
      aria-label={session.title}
      onClick={open}
      onKeyDown={onTileKey}
      className={cn(
        "group relative flex cursor-pointer flex-col overflow-hidden rounded-lg border bg-surface",
        "transition-colors duration-150",
        needsInput
          ? "border-primary/60 hover:border-primary"
          : cn("border-outline-variant", tint?.ring && `hover:${tint.ring}`),
      )}
    >
      {/* Slot 1: top accent bar */}
      <div
        className={cn(
          "absolute left-0 right-0 top-0 h-1 transition-opacity",
          needsInput
            ? "bg-primary opacity-100"
            : tint
              ? cn(tint.bar, "opacity-0 group-hover:opacity-100")
              : "opacity-0",
        )}
        aria-hidden
      />

      <div className={cn("flex-1 p-4", needsInput && "pb-3")}>
        {/* Slots 2-4: header row */}
        <div className="flex items-start justify-between gap-2">
          <div className="flex items-center gap-2">
            <code className="font-label-caps text-label-caps text-on-surface-variant">
              {formatSessionId(session.id)}
            </code>
            <SessionPhaseChip phase={phase} closed={closed} />
            {!closed && (
              <HealthDot
                health={worstHealth(health)}
                name="An agent"
                hideWhenHealthy
              />
            )}
            {!session.rain_enabled && (
              <span
                className="shrink-0 rounded border border-primary/40 bg-primary/15 px-1.5 py-0.5 font-label-caps text-label-caps text-primary"
                title="Solo Brian — Rain disabled"
              >
                SOLO
              </span>
            )}
          </div>
          <span className="shrink-0 font-code-sm text-code-sm text-on-surface-variant">
            {formatRelative(session.created_at)}
          </span>
        </div>

        {/* Slot 5: title */}
        <h3 className="mt-2 font-headline-md text-headline-md text-on-surface line-clamp-2">
          {session.title}
        </h3>

        {/* Slot 6: description (synthesized) */}
        <p className="mt-1 line-clamp-2 font-code-sm text-code-sm text-on-surface-variant">
          {describe(session)}
        </p>

        {/* Slot 7: pending-input indicator. The tile only INDICATES that this
            session has items awaiting the user (asks, gates, approvals); the
            user answers them on the session's Tray tab — the single answer
            surface — not inline here. */}
        {needsInput && (
          <div
            className="mt-3 inline-flex items-center gap-1.5 rounded border border-error/30 bg-error-container/20 px-2 py-1 font-label-caps text-label-caps text-error"
            title="Open the session's Tray tab to respond"
          >
            <AlertIcon />
            [Need User Input · {pendingCount}]
          </div>
        )}
      </div>

      {/* Slot 8: Quickview footer */}
      <div className="border-t border-outline-variant/30 px-4 py-2">
        <div className="font-label-caps text-label-caps text-on-surface-variant">
          Quickview
        </div>
        <div className="truncate font-code-sm text-code-sm italic text-on-surface">
          {quickviewFor(phase, closed)}
        </div>
      </div>
    </div>
  );
}

function formatSessionId(id: string): string {
  // sessions are spawned as `s-<8 hex>`; show `S-XXXX` to mirror the design.
  const cleaned = id.replace(/^s-/i, "");
  return `S-${cleaned.slice(0, 4).toUpperCase() || "????"}`;
}

function describe(session: SessionInfo): string {
  if (session.working_repo_path) {
    const basename =
      session.working_repo_path.split("/").filter(Boolean).pop() ?? "repo";
    return `Working in ${basename}`;
  }
  return `Created ${formatRelative(session.created_at)}`;
}

function quickviewFor(phase: string | null, closed: boolean): string {
  if (closed) return "Session closed";
  if (phase) {
    return `${phase.charAt(0).toUpperCase()}${phase.slice(1)} phase — open to view activity`;
  }
  return "Open session to view activity log";
}

function AlertIcon({ className }: { className?: string }) {
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
      <path d="M12 9v4" />
      <path d="M12 17h.01" />
      <path d="M10.29 3.86L1.82 18a2 2 0 001.71 3h16.94a2 2 0 001.71-3L13.71 3.86a2 2 0 00-3.42 0z" />
    </svg>
  );
}
