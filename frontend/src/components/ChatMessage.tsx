import { memo, useEffect, useMemo, useState } from "react";
import { Markdown } from "./Markdown";
import { authorColorClass } from "./authorColor";
import { cn } from "../lib/cn";
import { formatRelative } from "../lib/time";
import type { AgentMessage } from "../lib/bindings";

interface ChatMessageProps {
  message: AgentMessage;
  /** Hide the author header (consecutive messages from the same author). */
  groupedWithPrev?: boolean;
  /** Lifted tool-pill expand state, keyed by message id. Virtualized parents
   * own it (rows unmount when scrolled away, which would reset local state);
   * when omitted, ToolMessage falls back to its own local state. */
  expanded?: boolean;
  onToggleExpand?: (id: number) => void;
  /** `tool_use_id`s that already have a `tool_result`. A `tool_use` missing
   *  from this set is still RUNNING and renders as such. Omitted (undefined) =
   *  the parent doesn't track it, so nothing is claimed either way. */
  resolvedToolIds?: ReadonlySet<string>;
}

// Author + relative-timestamp header. Shared by the text and tool message rows
// (the markup was byte-identical) so the two can't drift.
function MessageHeader({
  author,
  createdAt,
}: {
  author: string;
  createdAt: string;
}) {
  return (
    <header className="mb-1 flex items-center gap-2">
      <span
        className={cn(
          "text-[0.65rem] font-semibold uppercase tracking-wide",
          authorColorClass(author),
        )}
      >
        {author}
      </span>
      <span className="text-[0.65rem] text-on-surface-variant">
        {formatRelative(createdAt)}
      </span>
    </header>
  );
}

/**
 * One rendered chat message. Markdown via react-markdown + GFM (tables,
 * task lists, autolinks). Code blocks get a contained scroll. Phase-change
 * messages render as centered muted-italic lines per the Industrial Terminal look.
 *
 * `groupedWithPrev` collapses the author header when the previous message
 * was from the same author — keeps long runs of agent output legible.
 */
export const ChatMessage = memo(function ChatMessage({
  message,
  groupedWithPrev,
  expanded,
  onToggleExpand,
  resolvedToolIds,
}: ChatMessageProps) {
  if (message.kind === "phase_change") {
    return (
      <div className="my-4 text-center text-[0.7rem] italic text-on-surface-variant">
        — {message.content} —
      </div>
    );
  }

  if (message.kind === "tool_use" || message.kind === "tool_result") {
    return (
      <ToolMessage
        message={message}
        groupedWithPrev={groupedWithPrev}
        expanded={expanded}
        onToggleExpand={onToggleExpand}
        resolvedToolIds={resolvedToolIds}
      />
    );
  }

  return (
    <article className={cn("mb-2", groupedWithPrev ? "mt-0" : "mt-3")}>
      {!groupedWithPrev && (
        <MessageHeader author={message.author} createdAt={message.created_at} />
      )}
      <Markdown>{message.content}</Markdown>
    </article>
  );
});

// Collapsible row for tool_use / tool_result messages. Raw JSON in the
// chat stream buries the agents' prose; this pulls them down to one-line
// pills with click-to-expand. Parses the storage-layer JSON shape:
//   tool_use:    { name, input | args, tool_use_id }
//   tool_result: { tool_use_id, output | content }
// Parser failures fall through to the raw content as a faint mono line.
function ToolMessage({
  message,
  groupedWithPrev,
  expanded: controlledExpanded,
  onToggleExpand,
  resolvedToolIds,
}: {
  message: AgentMessage;
  groupedWithPrev?: boolean;
  expanded?: boolean;
  onToggleExpand?: (id: number) => void;
  resolvedToolIds?: ReadonlySet<string>;
}) {
  const [localExpanded, setLocalExpanded] = useState(false);
  const controlled =
    controlledExpanded !== undefined && onToggleExpand !== undefined;
  const expanded = controlled ? controlledExpanded : localExpanded;
  const toggle = () =>
    controlled ? onToggleExpand(message.id) : setLocalExpanded((v) => !v);
  const parsed = safeParse(message.content);
  const isUse = message.kind === "tool_use";

  // Best-effort summary line.
  const toolName = isUse
    ? (parsed?.name as string | undefined) ?? "tool"
    : "result";
  const previewSource = isUse
    ? (parsed?.input ?? parsed?.args ?? parsed)
    : (parsed?.output ?? parsed?.content ?? parsed);
  const preview = formatPreview(previewSource, message.content);

  // Still running: a tool_use whose result hasn't landed. Only claimed when the
  // parent actually tracks results — an undefined set means "unknown", and
  // guessing "running" on an old finished call would be its own lie.
  const toolUseId =
    typeof parsed?.tool_use_id === "string" ? parsed.tool_use_id : null;
  const running =
    isUse &&
    resolvedToolIds !== undefined &&
    toolUseId !== null &&
    !resolvedToolIds.has(toolUseId);

  return (
    <article className={cn("mb-1", groupedWithPrev ? "mt-0" : "mt-2")}>
      {!groupedWithPrev && (
        <MessageHeader author={message.author} createdAt={message.created_at} />
      )}
      <button
        type="button"
        onClick={toggle}
        aria-expanded={expanded}
        className={cn(
          "flex w-full items-start gap-2 rounded border border-outline-variant bg-surface px-2 py-1 text-left",
          "text-[0.7rem] text-on-surface-variant hover:bg-surface-container-high transition-colors",
        )}
        title={isUse ? "tool call" : "tool result"}
      >
        <span aria-hidden className="w-3 text-on-surface-variant">
          {expanded ? "▾" : "▸"}
        </span>
        <span
          className={cn(
            "shrink-0 font-mono",
            running ? "text-primary" : "text-on-surface",
          )}
        >
          {running ? "⟳" : isUse ? "→" : "←"} {toolName}
        </span>
        {running && <Elapsed since={message.created_at} />}
        {/* Wraps to two lines instead of clipping to one: a truncated command
            is exactly the "under the hood" complaint. `break-all` keeps long
            unbroken tokens (paths, URLs) from forcing horizontal scroll. */}
        <span className="flex-1 line-clamp-2 break-all font-mono text-on-surface-variant">
          {preview}
        </span>
      </button>
      {expanded && (
        <pre className="mt-1 whitespace-pre-wrap break-words rounded border border-outline-variant bg-surface-container-lowest px-3 py-2 font-mono text-[0.7rem] leading-relaxed text-on-surface">
          {(() => {
            try {
              return JSON.stringify(parsed ?? message.content, null, 2);
            } catch {
              return message.content;
            }
          })()}
        </pre>
      )}
    </article>
  );
}

function safeParse(raw: string): Record<string, unknown> | null {
  try {
    const v = JSON.parse(raw);
    return typeof v === "object" && v !== null
      ? (v as Record<string, unknown>)
      : null;
  } catch {
    return null;
  }
}

// Collapsed tool rows are the user's only view of what an agent is actually
// doing, so the preview has to be long enough to hold a real command. 80 cut
// most `cargo test … | tail -25`-shaped lines in half; the row wraps to two
// lines rather than clipping, so this can afford to be generous.
const PREVIEW_MAX = 200;

/**
 * Live "this has been running for N" counter for an unresolved tool call.
 *
 * Derived from the message's own `created_at`, never from mount time — the chat
 * pane is virtualized, so rows unmount and remount as the user scrolls, and a
 * mount-anchored timer would restart at 0 each time. Only the tick interval
 * restarts, which nothing can observe.
 */
function Elapsed({ since }: { since: string }) {
  const start = useMemo(() => new Date(since).getTime(), [since]);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, []);
  if (!Number.isFinite(start)) return null;
  const secs = Math.max(0, Math.floor((now - start) / 1000));
  const label =
    secs < 60 ? `${secs}s` : `${Math.floor(secs / 60)}m ${secs % 60}s`;
  return (
    <span className="shrink-0 font-mono tabular-nums text-primary">
      {label}
    </span>
  );
}

function formatPreview(value: unknown, fallback: string): string {
  if (value == null) return clip(fallback);
  // Smart extraction for common tool shapes the agents emit a lot.
  if (typeof value === "object" && value !== null) {
    const v = value as Record<string, unknown>;
    const known =
      (typeof v.command === "string" && v.command) ||
      (typeof v.file_path === "string" && v.file_path) ||
      (typeof v.path === "string" && v.path) ||
      (typeof v.pattern === "string" && v.pattern) ||
      (typeof v.url === "string" && v.url) ||
      (typeof v.text === "string" && v.text) ||
      (typeof v.description === "string" && v.description);
    if (known) return clip(known);
    try {
      return clip(JSON.stringify(value));
    } catch {
      return clip(String(value));
    }
  }
  return clip(String(value));
}

function clip(s: string): string {
  const single = s.replace(/\s+/g, " ").trim();
  return single.length > PREVIEW_MAX
    ? single.slice(0, PREVIEW_MAX - 1) + "…"
    : single;
}

