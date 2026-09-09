import { memo } from "react";
import { useNavigate } from "react-router-dom";
import { cn } from "../lib/cn";
import { formatRelative } from "../lib/time";
import {
  ATTENTION_IDLE_LABEL,
  ATTENTION_IDLE_TOOLTIP,
  ATTENTION_IDLE_UNFLAGGED,
} from "../lib/attention";
import type { SessionInfo } from "../lib/bindings";
import { SessionPhaseChip, phaseTintClasses } from "./SessionPhaseChip";
import { useHealthStore, worstHealth } from "../stores/health";
import { HealthDot } from "./HealthDot";
import { authorColorClass } from "./authorColor";
import { WarnIcon } from "./icons";
import { authorLabel } from "../lib/participants";

interface SessionTileProps {
  session: SessionInfo;
  /** Count of items awaiting the user for this session (durable tray). The tile
   *  only INDICATES — the user answers on the session's Tray tab. */
  pendingCount?: number;
  /** Current IPAV phase (lowercase) from `get_session_phase`. Null when unknown. */
  phase?: string | null;
  /** Participant key → `ROLE · Model`, from the session's roster (rc3 D10).
   *  Empty is safe, but it is not free: an author with no row reads as
   *  "Unknown participant", so the roster is what makes the tag informative. */
  authorLabels?: Record<string, string>;
  /** Label → hue token, from the same roster (rc3 D20 — a user-picked
   *  participant colour). Absent falls back to the label hash. */
  authorHues?: Record<string, string>;
}

const NO_LABELS: Record<string, string> = {};

function SessionTileImpl({
  session,
  pendingCount = 0,
  phase = null,
  authorLabels = NO_LABELS,
  authorHues,
}: SessionTileProps) {
  const navigate = useNavigate();
  const closed = session.closed_at !== null;
  const needsInput = pendingCount > 0;
  const tint = phaseTintClasses(phase, closed);
  // B2: session-level health dot (problem-only on the tile). Worst of the
  // session's agents.
  const health = useHealthStore((s) => s.bySession[session.id]);
  // Idle-unflagged watchdog: the session sat idle past grace with no question
  // or halt parked — it probably needs direction.
  const attention = useHealthStore((s) => s.attentionBySession[session.id]);
  // Slot 8 Quickview: first line of the latest text message (null until one exists).
  const quickview = firstLine(session.last_message);
  // …and who wrote it, as `ROLE · Model` (rc3 D10), tinted with the roster
  // hues the Dashboard threads in (round 8) — so a user-picked participant
  // colour (D20) reads the same on this byline as inside the session view.
  const quickviewAuthor = authorLabel(session.last_author, authorLabels);

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
          : cn("border-outline-variant", tint?.hoverRing),
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
            {!closed && attention === ATTENTION_IDLE_UNFLAGGED && (
              <span
                className="shrink-0 rounded border border-warning/50 bg-warning/15 px-1.5 py-0.5 font-label-caps text-label-caps text-warning"
                title={ATTENTION_IDLE_TOOLTIP}
              >
                {ATTENTION_IDLE_LABEL}
              </span>
            )}
            {!session.multi_participant && (
              <span
                className="shrink-0 rounded border border-primary/40 bg-primary/15 px-1.5 py-0.5 font-label-caps text-label-caps text-primary"
                title="One participant — this session runs a single agent"
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
            <WarnIcon size={14} />
            [Need User Input · {pendingCount}]
          </div>
        )}
      </div>

      {/* Slot 8: Quickview footer — first line of the latest text message so a
          session can be monitored from the dashboard without opening it. Falls
          back to a generic phase hint until the session has a text message. */}
      <div className="border-t border-outline-variant/30 px-4 py-2">
        <div className="font-label-caps text-label-caps text-on-surface-variant">
          Quickview
        </div>
        {quickview ? (
          <div className="truncate font-code-sm text-code-sm text-on-surface">
            <span
              className={cn(
                "mr-1.5 font-semibold",
                authorColorClass(quickviewAuthor, authorHues),
              )}
            >
              {quickviewAuthor}
            </span>
            {quickview}
          </div>
        ) : (
          <div className="truncate font-code-sm text-code-sm italic text-on-surface-variant">
            {quickviewFor(phase, closed)}
          </div>
        )}
      </div>
    </div>
  );
}

// Memoized: the dashboard re-renders the whole tile grid on store/query churn;
// a tile only needs to re-render when its own props (session/pendingCount/phase)
// change.
export const SessionTile = memo(SessionTileImpl);

function formatSessionId(id: string): string {
  // sessions are spawned as `s-<8 hex>`; show `S-XXXX` to mirror the design.
  const cleaned = id.replace(/^s-/i, "");
  return `S-${cleaned.slice(0, 4).toUpperCase() || "????"}`;
}

function describe(session: SessionInfo): string {
  const created = `created ${formatRelative(session.created_at)}`;
  if (session.working_repo_path) {
    const basename =
      session.working_repo_path.split("/").filter(Boolean).pop() ?? "repo";
    const worktree = session.base_repo_path ? " · worktree" : "";
    return `${basename}${worktree} · ${created}`;
  }
  return `no repo · ${created}`;
}

function quickviewFor(phase: string | null, closed: boolean): string {
  if (closed) return "Session closed";
  if (phase) {
    return `${phase.charAt(0).toUpperCase()}${phase.slice(1)} phase — open to view activity`;
  }
  return "Open session to view activity log";
}

/** First non-blank line of a message body, trimmed; null when empty/absent. */
function firstLine(text: string | null): string | null {
  if (!text) return null;
  const line = text.split("\n").find((l) => l.trim().length > 0);
  return line ? line.trim() : null;
}

