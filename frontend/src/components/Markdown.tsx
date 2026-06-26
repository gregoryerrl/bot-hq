import { memo } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { cn } from "../lib/cn";

interface MarkdownProps {
  /** Raw markdown source to render. */
  children: string;
  /** Extra classes merged onto the wrapper. */
  className?: string;
}

/**
 * Shared markdown renderer styled for the Industrial Terminal look. Extracted
 * from ChatMessage so the chat stream and the IPAV DocumentPane render
 * identical markdown — GFM tables, task lists, autolinks; code blocks get a
 * contained scroll; links open in a new tab.
 */
export const Markdown = memo(function Markdown({
  children,
  className,
}: MarkdownProps) {
  return (
    <div className={cn("prose-tight text-sm text-on-surface", className)}>
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={markdownComponents}>
        {children}
      </ReactMarkdown>
    </div>
  );
});

const markdownComponents: Components = {
  p: ({ children }) => (
    <p className="mb-2 whitespace-pre-wrap leading-relaxed last:mb-0">
      {children}
    </p>
  ),
  code: ({ className, children, ...props }) => {
    const isBlock = (props as { node?: { tagName?: string } }).node?.tagName ===
      "code"
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
};
