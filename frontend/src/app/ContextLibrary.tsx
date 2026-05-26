import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTauriQuery } from "../hooks/useInvoke";
import { Input } from "../components/ui/Input";
import { Card } from "../components/ui/Card";
import { Button } from "../components/ui/Button";
import type { ClIndexEntryView, ClRescanReportView } from "../lib/bindings";

export function ContextLibrary() {
  const [project, setProject] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [rescanning, setRescanning] = useState(false);
  const [rescanReport, setRescanReport] = useState<ClRescanReportView | null>(
    null,
  );

  const { data: entries = [], isLoading, refetch } = useTauriQuery<ClIndexEntryView[]>(
    "cl_index_search",
    {
      project,
      query: query.trim() || null,
    },
  );

  const handleRescan = async () => {
    if (!project) return;
    setRescanning(true);
    setRescanReport(null);
    try {
      const report = await invoke<ClRescanReportView>("cl_rescan", { project });
      setRescanReport(report);
      refetch();
    } finally {
      setRescanning(false);
    }
  };

  const byProject = entries.reduce<Record<string, ClIndexEntryView[]>>(
    (acc, e) => {
      acc[e.project_id] = acc[e.project_id] ?? [];
      acc[e.project_id].push(e);
      return acc;
    },
    {},
  );

  return (
    <div className="mx-auto h-full max-w-6xl px-6 py-6">
      <div className="mb-4 flex items-center justify-between">
        <h1 className="text-xl font-semibold">Context Library</h1>
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
          className="w-48"
        />
        <Button
          variant="secondary"
          size="sm"
          onClick={handleRescan}
          disabled={!project || rescanning}
          title={project ? `cl_rescan(${project})` : "Set a project filter to rescan"}
        >
          {rescanning ? "Rescanning…" : "Rescan disk"}
        </Button>
      </div>
      {rescanReport && (
        <div className="mb-4 rounded border border-neutral-800 bg-neutral-900/60 px-3 py-2 text-xs text-neutral-300">
          Rescan complete · added {rescanReport.added.length} · touched{" "}
          {rescanReport.touched.length} · orphaned {rescanReport.orphaned.length}
        </div>
      )}
      {isLoading ? (
        <p className="text-sm text-neutral-500">Loading…</p>
      ) : entries.length === 0 ? (
        <p className="text-sm text-neutral-500">No matches.</p>
      ) : (
        <div className="space-y-6">
          {Object.entries(byProject).map(([projectId, files]) => (
            <section key={projectId}>
              <h2 className="mb-2 text-xs font-semibold uppercase tracking-wide text-neutral-400">
                {projectId} ({files.length})
              </h2>
              <div className="space-y-2">
                {files.map((f) => (
                  <Card key={f.id}>
                    <div className="flex items-center justify-between gap-2">
                      <code className="text-xs text-neutral-200">
                        {f.file_path}
                      </code>
                      <span className="text-[0.65rem] text-neutral-500">
                        {f.updated_at}
                      </span>
                    </div>
                    {f.description && (
                      <p className="mt-1 text-xs text-neutral-400">
                        {f.description}
                      </p>
                    )}
                  </Card>
                ))}
              </div>
            </section>
          ))}
        </div>
      )}
    </div>
  );
}
