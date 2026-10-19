import { memo } from "react";
import { Markdown } from "./Markdown";
import { authorColorClass } from "./authorColor";
import { cn } from "../lib/cn";
import { RelativeTime } from "./RelativeTime";
import { UNKNOWN_PARTICIPANT } from "../lib/participants";
import type { AgentMessage } from "../lib/bindings";
import { WarnIcon } from "./icons";

interface ChatMessageProps {
  message: AgentMessage;
  /** Hide the author header (consecutive messages from the same author). */
  groupedWithPrev?: boolean;
  /** What to PRINT for `message.author`. rc3 D10: the stored author is a
   *  participant slug and is never shown; the header renders this instead. */
  authorLabel?: string;
  /** Roster slot -> hue (rc3 D20). */
  authorHues?: Record<string, string>;
}

/**
 * Author + relative-timestamp header, shared by the message rows and the tool
 * bursts (`ToolBurst`) so the two cannot drift.
 *
 * `label` is what the user reads AND what picks the colour (rc3 D10). The
 * stored `message.author` slug reaches neither: printing it was the fallback
 * that put "brian" back on screen for every legacy row, and colouring by it
 * went neutral for every role-derived slug. No label at all means no roster was
 * consulted, which is what `UNKNOWN_PARTICIPANT` says.
 */
export function MessageHeader({
  label,
  createdAt,
  hues,
}: {
  label?: string;
  createdAt: string;
  /** Roster slot -> hue (rc3 D20). Absent falls back to the label hash. */
  hues?: Record<string, string>;
}) {
  const shown = label || UNKNOWN_PARTICIPANT;
  return (
    <header className="mb-1 flex items-center gap-2">
      <span className={cn("font-label-caps text-label-caps", authorColorClass(shown, hues))}>{shown}</span>
      <RelativeTime iso={createdAt} className="font-code-sm text-code-sm text-on-surface-variant" />
    </header>
  );
}

/**
 * One rendered chat message. Markdown via react-markdown + GFM (tables,
 * task lists, autolinks). Code blocks get a contained scroll. Phase-change
 * messages render as centered muted-italic lines per the Industrial Terminal look.
 * Tool rows never come here: `chatRows` folds them into bursts (`ToolBurst`).
 *
 * `groupedWithPrev` collapses the author header when the previous row was
 * from the same author — keeps long runs of agent output legible.
 */
export const ChatMessage = memo(function ChatMessage({
  message,
  groupedWithPrev,
  authorLabel,
  authorHues,
}: ChatMessageProps) {
  if (message.kind === "phase_change") {
    return (
      <div className="my-4 text-center font-code-sm text-code-sm italic text-on-surface-variant">
        — {message.content} —
      </div>
    );
  }

  if (message.kind === "system_notice") {
    // Host-emitted notice (e.g. the idle-unflagged watchdog). Centered like a
    // phase change — it is persisted with synthetic author=user, so it must
    // NOT fall through to the prose render and read as the user's words.
    return (
      <div className="my-4 text-center font-code-sm text-code-sm italic text-warning">
        <WarnIcon className="mr-1 inline-block size-3 align-middle" />
        {message.content}
      </div>
    );
  }

  return (
    <article className={cn("mb-2", groupedWithPrev ? "mt-0" : "mt-3")}>
      {!groupedWithPrev && (
        <MessageHeader label={authorLabel} createdAt={message.created_at} hues={authorHues} />
      )}
      <Markdown>{message.content}</Markdown>
    </article>
  );
});
