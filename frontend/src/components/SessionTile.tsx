import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { cn } from "../lib/cn";
import type { PendingChoiceView, SessionInfo } from "../lib/bindings";
import { SessionPhaseChip, phaseTintClasses } from "./SessionPhaseChip";

export interface SessionTileProps {
  session: SessionInfo;
  /** Pending choices pre-filtered to this session. First entry renders inline. */
  pendingChoices?: PendingChoiceView[];
  /** Current IPAV phase (lowercase) from `get_session_phase`. Null when unknown. */
  phase?: string | null;
}

export function SessionTile({
  session,
  pendingChoices = [],
  phase = null,
}: SessionTileProps) {
  const navigate = useNavigate();
  const closed = session.closed_at !== null;
  const needsInput = pendingChoices.length > 0;
  const firstChoice = pendingChoices[0];
  const tint = phaseTintClasses(phase, closed);

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

        {/* Slot 7: [Need User Input] pill */}
        {needsInput && (
          <div className="mt-3 inline-flex items-center gap-1.5 rounded border border-error/30 bg-error-container/20 px-2 py-1 font-label-caps text-label-caps text-error">
            <AlertIcon />
            [Need User Input]
          </div>
        )}
      </div>

      {/* Slot 8: inline ask_user_choice banner */}
      {needsInput && firstChoice && (
        <ChoiceBanner
          choice={firstChoice}
          extraCount={pendingChoices.length - 1}
        />
      )}

      {/* Slot 9: Quickview footer */}
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

interface ChoiceBannerProps {
  choice: PendingChoiceView;
  extraCount: number;
}

function ChoiceBanner({ choice, extraCount }: ChoiceBannerProps) {
  const [other, setOther] = useState("");
  const [submitting, setSubmitting] = useState(false);

  const submit = async (picked: string) => {
    if (submitting || !picked.trim()) return;
    setSubmitting(true);
    try {
      await invoke("resolve_choice", {
        choiceId: choice.choice_id,
        picked,
      });
      setOther("");
    } catch (err) {
      // eslint-disable-next-line no-console
      console.error("resolve_choice failed", err);
    } finally {
      setSubmitting(false);
    }
  };

  const isBinary = choice.options.length === 2;

  return (
    <div
      className="border-t border-outline-variant/50 bg-surface-container-low p-3"
      onClick={(e) => e.stopPropagation()}
      onKeyDown={(e) => e.stopPropagation()}
    >
      <div className="mb-2 font-code-sm text-code-sm text-on-surface">
        {choice.question}
      </div>

      {isBinary ? (
        <div className="flex gap-2">
          {choice.options.map((opt, i) => (
            <button
              key={opt}
              type="button"
              disabled={submitting}
              onClick={() => submit(opt)}
              className={cn(
                "flex-1 rounded border py-1 font-code-sm text-code-sm transition-colors disabled:opacity-50",
                i === 0
                  ? "border-primary/50 bg-primary/20 text-primary hover:bg-primary/30"
                  : "border-outline-variant bg-surface-variant text-on-surface hover:bg-surface-container-high",
              )}
            >
              {opt}
            </button>
          ))}
        </div>
      ) : (
        <div className="space-y-2">
          {choice.options.map((opt) => (
            <button
              key={opt}
              type="button"
              disabled={submitting}
              onClick={() => submit(opt)}
              className="group/btn flex w-full items-center justify-between rounded border border-outline-variant bg-surface-container px-3 py-2 font-code-sm text-code-sm text-on-surface transition-colors hover:bg-surface-variant disabled:opacity-50"
            >
              <span className="text-left">{opt}</span>
              <ChevronIcon className="text-primary opacity-0 transition-opacity group-hover/btn:opacity-100" />
            </button>
          ))}
        </div>
      )}

      <input
        type="text"
        value={other}
        onChange={(e) => setOther(e.target.value)}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === "Enter") submit(other);
        }}
        placeholder="Other:"
        disabled={submitting}
        className="mt-2 w-full rounded border border-outline-variant bg-surface-container-lowest px-2 py-1 font-code-sm text-code-sm text-on-surface placeholder:text-on-surface-variant focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary disabled:opacity-50"
      />

      {extraCount > 0 && (
        <div className="mt-2 font-label-caps text-label-caps text-on-surface-variant">
          +{extraCount} more pending
        </div>
      )}
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

function formatRelative(iso: string): string {
  if (!iso) return "";
  const then = new Date(iso).getTime();
  if (!Number.isFinite(then)) return iso;
  const now = Date.now();
  const sec = Math.max(0, Math.floor((now - then) / 1000));
  if (sec < 60) return `${sec}s ago`;
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const day = Math.floor(hr / 24);
  return `${day}d ago`;
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

function ChevronIcon({ className }: { className?: string }) {
  return (
    <svg
      className={cn("size-4", className)}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <path d="M9 18l6-6-6-6" />
    </svg>
  );
}
