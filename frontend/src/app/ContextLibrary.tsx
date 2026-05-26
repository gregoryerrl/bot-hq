import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTauriQuery } from "../hooks/useInvoke";
import { Input } from "../components/ui/Input";
import { Button } from "../components/ui/Button";
import { cn } from "../lib/cn";
import type { ClIndexEntryView, ClRescanReportView } from "../lib/bindings";

export function ContextLibrary() {
  const [project, setProject] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  // 300ms debounce — input value updates instantly for keystroke feedback;
  // the Tauri call uses the settled value so we don't hammer the bridge.
  const [debouncedQuery, setDebouncedQuery] = useState("");
  useEffect(() => {
    const id = setTimeout(() => setDebouncedQuery(query), 300);
    return () => clearTimeout(id);
  }, [query]);
  const [rescanning, setRescanning] = useState(false);
  const [rescanReport, setRescanReport] = useState<ClRescanReportView | null>(
    null,
  );
  const [collapsedProjects, setCollapsedProjects] = useState<Set<string>>(
    new Set(),
  );

  const {
    data: entries = [],
    isLoading,
    refetch,
  } = useTauriQuery<ClIndexEntryView[]>("cl_index_search", {
    project,
    query: debouncedQuery.trim() || null,
  });

  const byProject = useMemo(() => {
    const acc: Record<string, ClIndexEntryView[]> = {};
    for (const e of entries) {
      (acc[e.project_id] = acc[e.project_id] ?? []).push(e);
    }
    // Sort each project's files by path for stable visual order.
    for (const k of Object.keys(acc)) {
      acc[k].sort((a, b) => a.file_path.localeCompare(b.file_path));
    }
    return acc;
  }, [entries]);

  const handleRescan = async () => {
    if (rescanning) return;
    setRescanning(true);
    setRescanReport(null);
    try {
      if (project) {
        // Single-project rescan
        const report = await invoke<ClRescanReportView>("cl_rescan", {
          project,
        });
        setRescanReport(report);
      } else {
        // All-projects rescan: iterate over every project we know about.
        // NOTE: derived from current `byProject` (which comes from
        // `cl_index_search` results). If a project has zero indexed files
        // AND is filtered out by the search query, it won't be included.
        // Acceptable for the common "clear search, then rescan all" flow.
        const projectIds = Object.keys(byProject);
        const agg: ClRescanReportView = {
          added: [],
          touched: [],
          orphaned: [],
        };
        for (const p of projectIds) {
          try {
            const r = await invoke<ClRescanReportView>("cl_rescan", {
              project: p,
            });
            agg.added.push(...r.added);
            agg.touched.push(...r.touched);
            agg.orphaned.push(...r.orphaned);
          } catch (e) {
            // One bad project shouldn't kill the whole sweep.
            // eslint-disable-next-line no-console
            console.warn(`cl_rescan(${p}) failed`, e);
          }
        }
        setRescanReport(agg);
      }
      refetch();
    } finally {
      setRescanning(false);
    }
  };

  const toggleProject = (id: string) => {
    setCollapsedProjects((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  return (
    <div className="mx-auto h-full max-w-6xl overflow-auto px-6 py-6">
      <div className="mb-4 flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold tracking-tight">
            Context Library
          </h1>
          <p className="mt-1 text-xs text-neutral-500">
            {entries.length} entries across {Object.keys(byProject).length}{" "}
            project{Object.keys(byProject).length === 1 ? "" : "s"}
          </p>
        </div>
        <Button
          variant="secondary"
          size="sm"
          onClick={() => refetch()}
          title="Refetch search results"
        >
          ↻ Refresh
        </Button>
      </div>
      <div className="mb-4 flex gap-2">
        <div className="relative flex-1">
          <Input
            placeholder="Search CL files (substring on path/description/tags)…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            // Right padding leaves room for the clear button overlay.
            className="w-full pr-8"
          />
          {query.length > 0 && (
            <button
              type="button"
              onClick={() => setQuery("")}
              aria-label="Clear search"
              title="Clear search"
              className="absolute inset-y-0 right-0 flex w-8 items-center justify-center text-neutral-500 hover:text-neutral-100"
            >
              ×
            </button>
          )}
        </div>
        <Input
          placeholder="project filter (blank = all)"
          value={project ?? ""}
          onChange={(e) => setProject(e.target.value || null)}
          className="w-56"
        />
        <Button
          variant="secondary"
          size="sm"
          onClick={handleRescan}
          disabled={
            rescanning ||
            (!project && Object.keys(byProject).length === 0)
          }
          title={
            project
              ? `cl_rescan(${project})`
              : `cl_rescan all (${Object.keys(byProject).length} project${
                  Object.keys(byProject).length === 1 ? "" : "s"
                })`
          }
        >
          {rescanning
            ? "Rescanning…"
            : project
              ? "Rescan disk"
              : `Rescan all (${Object.keys(byProject).length})`}
        </Button>
      </div>
      {rescanReport && (
        <div className="mb-4 flex flex-wrap gap-3 rounded border border-default bg-surface px-3 py-2 text-xs">
          <span className="text-neutral-300">Rescan complete</span>
          <span className="text-emerald-300">
            + {rescanReport.added.length} added
          </span>
          <span className="text-blue-300">
            ↻ {rescanReport.touched.length} touched
          </span>
          <span className="text-amber-300">
            ⚠ {rescanReport.orphaned.length} orphaned
          </span>
        </div>
      )}
      {isLoading ? (
        <div className="space-y-2">
          {[0, 1, 2, 3].map((i) => (
            <div
              key={i}
              className="h-12 animate-pulse rounded border border-default bg-surface"
            />
          ))}
        </div>
      ) : entries.length === 0 ? (
        <div className="rounded-lg border border-dashed border-default p-12 text-center text-sm text-neutral-400">
          {query.trim() || project
            ? "No matches for that search."
            : "CL index is empty. Use `Rescan disk` for a specific project to populate it."}
        </div>
      ) : (
        <div className="space-y-4">
          {Object.entries(byProject).map(([projectId, files]) => {
            const collapsed = collapsedProjects.has(projectId);
            return (
              <section
                key={projectId}
                className="rounded-lg border border-default bg-surface"
              >
                <button
                  onClick={() => toggleProject(projectId)}
                  aria-expanded={!collapsed}
                  className="flex w-full items-center justify-between border-b border-default px-3 py-2 text-left text-xs font-semibold uppercase tracking-wide text-neutral-300 hover:bg-elevated"
                >
                  <span className="flex items-center gap-2">
                    <span
                      className={cn(
                        "inline-block w-3 text-neutral-500 transition-transform",
                        collapsed ? "" : "rotate-90",
                      )}
                    >
                      ▸
                    </span>
                    {projectId}
                  </span>
                  <span className="text-[0.65rem] font-normal text-neutral-500">
                    {files.length} file{files.length === 1 ? "" : "s"}
                  </span>
                </button>
                {!collapsed && (
                  <div className="divide-y divide-subtle">
                    {files.map((f) => (
                      <div
                        key={f.id}
                        className="px-3 py-2 transition-colors hover:bg-elevated"
                      >
                        <div className="flex items-start justify-between gap-2">
                          <code className="font-mono text-xs text-neutral-200">
                            {f.file_path}
                          </code>
                          <span
                            className="shrink-0 text-[0.65rem] text-neutral-500"
                            title={f.updated_at}
                          >
                            {formatRelative(f.updated_at)}
                          </span>
                        </div>
                        {f.description && (
                          <p className="mt-1 text-xs leading-relaxed text-neutral-400">
                            {f.description}
                          </p>
                        )}
                        {f.tags && (
                          <p className="mt-1 text-[0.65rem] text-neutral-600">
                            tags: {f.tags}
                          </p>
                        )}
                      </div>
                    ))}
                  </div>
                )}
              </section>
            );
          })}
        </div>
      )}
    </div>
  );
}

// Coarse human-readable "time since" matching SessionTile's idiom but
// extended for older entries: CL files can be months/years old.
// Hover title keeps the precise ISO timestamp accessible.
function formatRelative(iso: string): string {
  if (!iso) return "";
  const then = new Date(iso).getTime();
  if (!Number.isFinite(then)) return iso.slice(0, 10);
  const now = Date.now();
  const sec = Math.max(0, Math.floor((now - then) / 1000));
  if (sec < 60) return "just now";
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const day = Math.floor(hr / 24);
  if (day === 0) return "today";
  if (day === 1) return "yesterday";
  if (day < 7) return `${day}d ago`;
  const week = Math.floor(day / 7);
  if (week < 5) return `${week}w ago`;
  const month = Math.floor(day / 30);
  if (month < 12) return `${month}mo ago`;
  const year = Math.floor(day / 365);
  return `${year}y ago`;
}
