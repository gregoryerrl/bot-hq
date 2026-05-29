import { memo, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { authorColorClass } from "./authorColor";
import { cn } from "../lib/cn";
import type { AgentMessage } from "../lib/bindings";

interface ChatMessageProps {
  message: AgentMessage;
  /** Hide the author header (consecutive messages from the same author). */
  groupedWithPrev?: boolean;
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
        <header className="mb-1 flex items-center gap-2">
          <span
            className={cn(
              "text-[0.65rem] font-semibold uppercase tracking-wide",
              authorColorClass(message.author),
            )}
          >
            {message.author}
          </span>
          <span className="text-[0.65rem] text-on-surface-variant">
            {formatRelative(message.created_at)}
          </span>
        </header>
      )}
      <div className="prose-tight text-sm text-on-surface">
        <ReactMarkdown
          remarkPlugins={[remarkGfm]}
          components={{
            p: ({ children }) => (
              <p className="mb-2 whitespace-pre-wrap leading-relaxed last:mb-0">
                {children}
              </p>
            ),
            code: ({ className, children, ...props }) => {
              const isBlock = (props as { node?: { tagName?: string } }).node
                ?.tagName === "code"
                ? !(className ?? "").includes("language-") &&
                  String(children).indexOf("\n") < 0
                  ? false
                  : true
                : false;
              if (isBlock) {
                return (
                  <pre className="my-2 overflow-x-auto rounded border border-outline-variant bg-surface-container-lowest px-3 py-2 font-mono text-[0.75rem] leading-relaxed text-on-surface">
                    <code className={className}>{children}</code>
                  </pre>
                );
              }
              return (
                <code className="rounded bg-surface-container-high px-1 py-0.5 font-mono text-[0.78rem] text-on-surface">
                  {children}
                </code>
              );
            },
            pre: ({ children }) => <>{children}</>,
            a: ({ href, children }) => (
              <a
                href={href}
                target="_blank"
                rel="noreferrer"
                className="text-tertiary underline hover:text-tertiary"
              >
                {children}
              </a>
            ),
            ul: ({ children }) => (
              <ul className="my-2 ml-5 list-disc space-y-1">{children}</ul>
            ),
            ol: ({ children }) => (
              <ol className="my-2 ml-5 list-decimal space-y-1">{children}</ol>
            ),
            li: ({ children }) => <li className="leading-relaxed">{children}</li>,
            blockquote: ({ children }) => (
              <blockquote className="my-2 border-l-2 border-outline-variant pl-3 italic text-on-surface">
                {children}
              </blockquote>
            ),
            h1: ({ children }) => (
              <h1 className="my-2 text-base font-semibold">{children}</h1>
            ),
            h2: ({ children }) => (
              <h2 className="my-2 text-sm font-semibold">{children}</h2>
            ),
            h3: ({ children }) => (
              <h3 className="my-2 text-sm font-semibold">{children}</h3>
            ),
          }}
        >
          {message.content}
        </ReactMarkdown>
      </div>
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
        <header className="mb-1 flex items-center gap-2">
          <span
            className={cn(
              "text-[0.65rem] font-semibold uppercase tracking-wide",
              authorColorClass(message.author),
            )}
          >
            {message.author}
          </span>
          <span className="text-[0.65rem] text-on-surface-variant">
            {formatRelative(message.created_at)}
          </span>
        </header>
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
        <pre className="mt-1 overflow-x-auto rounded border border-outline-variant bg-surface-container-lowest px-3 py-2 font-mono text-[0.7rem] leading-relaxed text-on-surface">
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

function formatRelative(iso: string): string {
  if (!iso) return "";
  const then = new Date(iso).getTime();
  if (!Number.isFinite(then)) return "";
  const now = Date.now();
  const sec = Math.max(0, Math.floor((now - then) / 1000));
  if (sec < 60) return "just now";
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const day = Math.floor(hr / 24);
  return `${day}d ago`;
}
