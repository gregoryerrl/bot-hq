import { memo } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { authorColorClass } from "./AuthorBadge";
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
 * messages render as centered muted-italic lines per the Slint-era look.
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
      <div className="my-4 text-center text-[0.7rem] italic text-neutral-500">
        — {message.content} —
      </div>
    );
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
          <span className="text-[0.65rem] text-neutral-600">
            {formatRelative(message.created_at)}
          </span>
        </header>
      )}
      <div className="prose-tight text-sm text-neutral-100">
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
                  <pre className="my-2 overflow-x-auto rounded border border-default bg-canvas px-3 py-2 font-mono text-[0.75rem] leading-relaxed text-neutral-200">
                    <code className={className}>{children}</code>
                  </pre>
                );
              }
              return (
                <code className="rounded bg-elevated px-1 py-0.5 font-mono text-[0.78rem] text-neutral-200">
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
                className="text-blue-400 underline hover:text-blue-300"
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
              <blockquote className="my-2 border-l-2 border-default pl-3 italic text-neutral-300">
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
