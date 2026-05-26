import { useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTauriQuery } from "../hooks/useInvoke";
import { Input } from "../components/ui/Input";
import { Button } from "../components/ui/Button";
import { cn } from "../lib/cn";
import type { ClIndexEntryView, ClRescanReportView } from "../lib/bindings";

export function ContextLibrary() {
  const [project, setProject] = useState<string | null>(null);
  const [query, setQuery] = useState("");
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
    query: query.trim() || null,
  });

  const handleRescan = async () => {
    if (!project) return;
    setRescanning(true);
    setRescanReport(null);
    try {
      const report = await invoke<ClRescanReportView>("cl_rescan", {
        project,
      });
      setRescanReport(report);
      refetch();
    } finally {
      setRescanning(false);
    }
  };

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
        <Input
          placeholder="Search CL files (substring on path/description/tags)…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          className="flex-1"
        />
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
          disabled={!project || rescanning}
          title={
            project
              ? `cl_rescan(${project})`
              : "Set a project filter to rescan a specific project"
          }
        >
          {rescanning ? "Rescanning…" : "Rescan disk"}
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
                          <span className="shrink-0 text-[0.65rem] text-neutral-500">
                            {f.updated_at.slice(0, 10)}
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
