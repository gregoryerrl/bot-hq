import { useEffect, useMemo, useState } from "react";
import { parseUtcMs } from "../lib/time";
import {
  burstFailures,
  burstTools,
  firstLine,
  formatSpan,
  type BurstRow,
  type ToolCall,
} from "../lib/chatRows";
import { fileArgInCommand } from "./FileViewerDialog";
import { CheckIcon, ChevronDownIcon, ChevronRightIcon, CrossIcon } from "./icons";
import { Button } from "./ui/Button";

/**
 * **One participant's run of tool calls as ONE quiet line** (the visual pass,
 * S3). Collapsed it says how many calls, which tools, how long the run took and
 * how many failed — `3 tools · Bash ×2, Read · 14s · 1 failed` — or, for a
 * single call, the call itself. Expanded it lists one human line per call (the
 * composer's own `describeToolCall`) with its result's first line, a mark for
 * passed / failed / running, the "View <file>" link for a call that names a
 * file, a thumbnail for an image result, and the raw call behind "raw".
 *
 * A failure is never hidden by the fold: the collapsed line carries the count
 * in the error tone. Notices are not part of a burst (`chatRows`), so a limit
 * or a gate stays a row of its own.
 */
export function ToolBurst({
  burst,
  expanded,
  onToggle,
  onViewFile,
}: {
  burst: BurstRow;
  expanded: boolean;
  onToggle: () => void;
  onViewFile?: (path: string) => void;
}) {
  const { calls } = burst;
  const failed = burstFailures(calls);
  const running = calls.filter(isRunning);
  const single = calls.length === 1 ? calls[0] : null;
  const span = formatSpan(parseUtcMs(burst.lastAt) - parseUtcMs(burst.firstAt));
  // Folded, a single call keeps its own View link (round 8), and a run of
  // calls offers the files it WROTE — what the user most wants to open —
  // rather than every path its reads and commands touched; open, every call
  // carries its own link.
  const folded = expanded ? [] : single ? [viewablePath(single)].filter(isPath) : writtenFiles(calls);

  return (
    <div className="mb-1 mt-1">
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={expanded}
        className="flex w-full items-start gap-2 rounded border border-outline-variant bg-surface px-2 py-1 text-left font-code-sm text-code-sm text-on-surface-variant transition-colors hover:bg-surface-container-high"
      >
        <span aria-hidden className="w-3 shrink-0">
          {expanded ? (
            <ChevronDownIcon className="mt-0.5 size-3" />
          ) : (
            <ChevronRightIcon className="mt-0.5 size-3" />
          )}
        </span>
        {/* `break-words`, never a sideways scroll or a one-line clip: a long
            command is what the user opens the fold to read. */}
        <span className="min-w-0 flex-1 break-words">
          {single ? (
            <CallText call={single} />
          ) : (
            <>
              <span className="text-on-surface">{calls.length} tools</span>
              {" · "}
              {burstTools(calls)}
            </>
          )}
          {running.length === 0 && calls.length > 1 && ` · ${span}`}
          {failed > 0 && <span className="text-error">{` · ${failed} failed`}</span>}
        </span>
        {running.length > 0 && running[0].use && (
          <span className="shrink-0 text-primary">
            running <Elapsed since={running[0].use.created_at} />
          </span>
        )}
      </button>
      {folded.length > 0 && onViewFile && (
        <div className="flex flex-wrap items-baseline gap-x-3">
          {folded.slice(0, FOLDED_FILES).map((path) => (
            <ViewFile key={path} path={path} onViewFile={onViewFile} />
          ))}
          {folded.length > FOLDED_FILES && (
            <span className="font-code-sm text-code-sm text-subtle">+{folded.length - FOLDED_FILES} more</span>
          )}
        </div>
      )}
      {expanded && (
        <ol className="mt-1 space-y-1 border-l border-outline-variant pl-3">
          {calls.map((call, i) => (
            <CallLine key={call.id ?? i} call={call} onViewFile={onViewFile} />
          ))}
        </ol>
      )}
    </div>
  );
}

const FOLDED_FILES = 3;
const WRITERS = new Set(["Write", "Edit", "MultiEdit", "NotebookEdit"]);

const isPath = (p: string | null): p is string => p !== null;

/** The files a run of calls wrote or edited, once each, in order. */
function writtenFiles(calls: readonly ToolCall[]): string[] {
  return [...new Set(calls.filter((c) => WRITERS.has(c.tool)).map(viewablePath).filter(isPath))];
}

/** A call is running while its result has not landed — claimed only for a
 *  call with an id, the one thing a result can be matched by: guessing
 *  "running" for a row that carries none would be a fresh lie. */
function isRunning(call: ToolCall): boolean {
  return call.use !== null && call.id !== null && call.result === null;
}

/** A call as text: its tool, then what it did — the label without the tool's
 *  name when the label already starts with it (`Read lib/x.ts`). */
function CallText({ call }: { call: ToolCall }) {
  const rest = call.label.startsWith(call.tool) ? call.label.slice(call.tool.length).trim() : call.label;
  return (
    <>
      <span className="text-on-surface">{call.tool}</span>
      {rest && ` · ${rest}`}
    </>
  );
}

function CallLine({ call, onViewFile }: { call: ToolCall; onViewFile?: (path: string) => void }) {
  const [raw, setRaw] = useState(false);
  const path = viewablePath(call);
  const summary = call.result ? firstLine(call.text) : "";
  return (
    <li className="font-code-sm text-code-sm text-on-surface-variant">
      <div className="flex items-start gap-2">
        <StatusMark call={call} />
        <span className="min-w-0 flex-1 break-words">
          <CallText call={call} />
          {summary && (
            <>
              {" — "}
              <span className={call.failed ? "text-error" : undefined}>{summary}</span>
            </>
          )}
        </span>
        <button
          type="button"
          onClick={() => setRaw((v) => !v)}
          aria-expanded={raw}
          className="shrink-0 text-subtle underline decoration-dotted underline-offset-2 hover:text-on-surface"
        >
          raw
        </button>
      </div>
      {path && onViewFile && <ViewFile path={path} onViewFile={onViewFile} />}
      {call.images.map((src, i) => (
        <img
          key={i}
          src={src}
          alt={`Image ${i + 1} returned by ${call.tool}`}
          className="mt-1 max-h-40 max-w-full rounded border border-outline-variant"
        />
      ))}
      {raw && (
        <pre className="mt-1 whitespace-pre-wrap break-words rounded border border-outline-variant bg-surface-container-lowest px-3 py-2 leading-relaxed text-on-surface">
          {rawText(call)}
        </pre>
      )}
    </li>
  );
}

function StatusMark({ call }: { call: ToolCall }) {
  if (isRunning(call)) {
    return (
      <span className="mt-0.5 flex size-3 shrink-0 items-center justify-center" title="running">
        <span className="size-1.5 animate-pulse rounded-full bg-primary" />
      </span>
    );
  }
  if (call.failed) {
    return (
      <span className="mt-0.5 shrink-0 text-error" title="failed">
        <CrossIcon className="size-3" />
      </span>
    );
  }
  return (
    <span className="mt-0.5 shrink-0 text-success" title="done">
      <CheckIcon className="size-3" />
    </span>
  );
}

function ViewFile({ path, onViewFile }: { path: string; onViewFile: (path: string) => void }) {
  return (
    <Button variant="link" onClick={() => onViewFile(path)} className="mt-0.5 font-code-sm text-code-sm" title={path}>
      View {path.split("/").pop()}
    </Button>
  );
}

/** The call as it ran: its arguments and its output text. An image is named,
 *  never dumped — a screenshot's base64 is no use to a reader. */
function rawText(call: ToolCall): string {
  const parts: string[] = [];
  if (call.use) {
    let input: string;
    try {
      input = JSON.stringify(call.input, null, 2);
    } catch {
      input = String(call.input);
    }
    parts.push(`${call.tool} ${input}`);
  }
  if (call.result) {
    parts.push(call.text === "" ? "(no output)" : call.text);
    if (call.images.length > 0) parts.push(`[${call.images.length} image${call.images.length > 1 ? "s" : ""}]`);
  }
  return parts.join("\n\n");
}

/**
 * The file a tool call's input names, if any: a `file_path` / `path` /
 * `notebook_path` argument, or the file argument of a Bash command
 * (`fileArgInCommand`). Prose paths are not linkified — the user's choice; the
 * args are structured, the prose is not (round 8).
 */
function viewablePath(call: ToolCall): string | null {
  const input = call.input;
  if (!call.use || typeof input !== "object" || input === null) return null;
  const v = input as Record<string, unknown>;
  for (const key of ["file_path", "path", "notebook_path"] as const) {
    const val = v[key];
    if (typeof val === "string" && val.trim() !== "") return val;
  }
  if (typeof v.command === "string") return fileArgInCommand(v.command);
  return null;
}

/**
 * Live "running for N" counter for a call still in flight. Derived from the
 * row's own `created_at`, never from mount time — the chat is virtualized, so
 * rows unmount and remount as the user scrolls, and a mount-anchored timer
 * would restart at 0. `parseUtcMs`, not `new Date`, so a zone-less timestamp
 * is not read as local (round 9).
 */
function Elapsed({ since }: { since: string }) {
  const start = useMemo(() => parseUtcMs(since), [since]);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, []);
  if (!Number.isFinite(start)) return null;
  return <span className="tabular-nums">{formatSpan(now - start)}</span>;
}
