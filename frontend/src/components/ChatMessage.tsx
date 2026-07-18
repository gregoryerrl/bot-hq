import { memo, useState } from "react";
import { Markdown } from "./Markdown";
import { authorColorClass } from "./authorColor";
import { cn } from "../lib/cn";
import { formatRelative } from "../lib/time";
import type { AgentMessage } from "../lib/bindings";

interface ChatMessageProps {
  message: AgentMessage;
  /** Hide the author header (consecutive messages from the same author). */
  groupedWithPrev?: boolean;
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
}: ChatMessageProps) {
  if (message.kind === "phase_change") {
    return (
      <div className="my-4 text-center text-[0.7rem] italic text-on-surface-variant">
        — {message.content} —
      </div>
    );
  }

  if (message.kind === "tool_use" || message.kind === "tool_result") {
    return <ToolMessage message={message} groupedWithPrev={groupedWithPrev} />;
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
}: {
  message: AgentMessage;
  groupedWithPrev?: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
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

  return (
    <article className={cn("mb-1", groupedWithPrev ? "mt-0" : "mt-2")}>
      {!groupedWithPrev && (
        <MessageHeader author={message.author} createdAt={message.created_at} />
      )}
      <button
        type="button"
        onClick={() => setExpanded((v) => !v)}
        aria-expanded={expanded}
        className={cn(
          "flex w-full items-center gap-2 rounded border border-outline-variant bg-surface px-2 py-1 text-left",
          "text-[0.7rem] text-on-surface-variant hover:bg-surface-container-high transition-colors",
        )}
        title={isUse ? "tool call" : "tool result"}
      >
        <span aria-hidden className="w-3 text-on-surface-variant">
          {expanded ? "▾" : "▸"}
        </span>
        <span className="font-mono text-on-surface">
          {isUse ? "→" : "←"} {toolName}
        </span>
        <span className="flex-1 truncate font-mono text-on-surface-variant">
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

const PREVIEW_MAX = 80;

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

