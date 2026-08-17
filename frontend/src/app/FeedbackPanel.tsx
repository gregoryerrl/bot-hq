import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { Link } from "react-router-dom";
import { useTauriQuery, errorMessage } from "../hooks/useInvoke";
import { ErrorBanner } from "../components/ErrorBanner";
import { Markdown } from "../components/Markdown";
import { Button } from "../components/ui/Button";
import { cn } from "../lib/cn";
import { formatRelative } from "../lib/time";
import { shortSessionId } from "../lib/sessionId";

/** Mirror of the Rust `AgentFeedbackView` — raw invoke, no binding needed. */
interface AgentFeedbackView {
  id: number;
  session_id: string;
  project: string | null;
  agent: string;
  kind: string;
  title: string;
  body: string;
  status: string;
  created_at: string;
  updated_at: string;
}

type Filter = "open" | "done" | "dismissed" | "all";

const FILTERS: Filter[] = ["open", "done", "dismissed", "all"];

/**
 * The read side of `file_feedback`: issues and ideas agents raised about bot-hq
 * ITSELF while working on other projects.
 *
 * `project` on a row is provenance — where the friction was hit — not the
 * subject, which is always bot-hq. Each row links back to the originating
 * session so the conversation that produced it is one click away.
 */
export function FeedbackPanel() {
  const queryClient = useQueryClient();
  const [filter, setFilter] = useState<Filter>("open");
  const [expanded, setExpanded] = useState<Set<number>>(new Set());
  const [error, setError] = useState<string | null>(null);

  // No event backs this query: agents file rows through the `file_feedback`
  // MCP tool, which emits nothing, and Settings keeps this panel mounted once
  // visited — so without a Refresh a row filed while the tab is open never
  // appeared for the rest of the app's run (round 8).
  const {
    data: rows = [],
    refetch,
    isFetching,
  } = useTauriQuery<AgentFeedbackView[]>("list_agent_feedback", {
    status: filter === "all" ? null : filter,
  });

  const setStatus = (id: number, status: string) => {
    setError(null);
    invoke<boolean>("set_agent_feedback_status", { id, status })
      .catch((e) => setError(errorMessage(e)))
      .finally(() => {
        void queryClient.invalidateQueries({ queryKey: ["list_agent_feedback"] });
      });
  };

  const toggle = (id: number) =>
    setExpanded((s) => {
      const next = new Set(s);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  return (
    <div className="h-full overflow-y-auto overflow-x-hidden px-4 py-3">
      <p className="mb-3 font-body-md text-body-md text-on-surface-variant">
        Issues and ideas the agents raised about bot-hq itself, filed from
        whatever project they were working on. Work them from a bot-hq session.
      </p>

      {error && (
        <ErrorBanner
          label="Couldn't update this item:"
          message={error}
          onDismiss={() => setError(null)}
          className="mb-3"
        />
      )}

      <div className="mb-3 flex items-center gap-1.5">
        {FILTERS.map((f) => (
          <Button
            key={f}
            size="sm"
            variant={filter === f ? "secondary" : "ghost"}
            onClick={() => setFilter(f)}
          >
            {f}
          </Button>
        ))}
        <button
          type="button"
          onClick={() => refetch()}
          className="ml-auto shrink-0 rounded border border-outline-variant px-2.5 py-1 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:text-on-surface"
        >
          {isFetching ? "Refreshing…" : "Refresh"}
        </button>
      </div>

      {rows.length === 0 ? (
        <p className="font-body-md text-body-md text-on-surface-variant">
          {filter === "open"
            ? "Nothing filed. Agents raise items here with file_feedback."
            : `No ${filter} items.`}
        </p>
      ) : (
        <ul className="space-y-2">
          {rows.map((f) => (
            <li
              key={f.id}
              className="rounded border border-outline-variant bg-surface-container px-3 py-2"
            >
              <div className="flex items-center gap-2">
                <span
                  className={cn(
                    "rounded px-1.5 py-0.5 text-[0.6rem] uppercase tracking-wide",
                    f.kind === "issue"
                      ? "bg-error-container/40 text-on-error-container"
                      : "bg-secondary/20 text-secondary",
                  )}
                >
                  {f.kind}
                </span>
                <button
                  type="button"
                  onClick={() => toggle(f.id)}
                  className="min-w-0 flex-1 truncate text-left font-body-md text-body-md text-on-surface hover:underline"
                  title={f.title}
                >
                  {f.title}
                </button>
                <span className="shrink-0 text-[0.7rem] text-on-surface-variant">
                  {f.agent} · {formatRelative(f.created_at) || "earlier"}
                </span>
              </div>

              {expanded.has(f.id) && (
                <div className="mt-2 border-t border-outline-variant pt-2">
                  <Markdown>{f.body}</Markdown>
                  <div className="mt-2 flex flex-wrap items-center gap-2 text-[0.7rem] text-on-surface-variant">
                    <span>
                      filed from{" "}
                      <Link
                        to={`/sessions/${f.session_id}`}
                        className="text-primary hover:underline"
                      >
                        {shortSessionId(f.session_id)}
                      </Link>
                      {f.project ? ` · ${f.project}` : ""}
                    </span>
                    <span className="ml-auto flex gap-1.5">
                      {f.status !== "done" && (
                        <Button size="sm" variant="ghost" onClick={() => setStatus(f.id, "done")}>
                          Mark done
                        </Button>
                      )}
                      {f.status !== "dismissed" && (
                        <Button
                          size="sm"
                          variant="ghost"
                          onClick={() => setStatus(f.id, "dismissed")}
                        >
                          Dismiss
                        </Button>
                      )}
                      {f.status !== "open" && (
                        <Button size="sm" variant="ghost" onClick={() => setStatus(f.id, "open")}>
                          Reopen
                        </Button>
                      )}
                    </span>
                  </div>
                </div>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
