import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTauriQuery } from "../hooks/useInvoke";
import { Input } from "../components/ui/Input";
import { Textarea } from "../components/ui/Textarea";
import { Button } from "../components/ui/Button";
import { cn } from "../lib/cn";
import type {
  ClFileContentView,
  ClIndexEntryView,
  ClRescanReportView,
  ProjectView,
} from "../lib/bindings";

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
  // Persist expand/collapse choices across route navigation + restarts.
  // Lazy-init from localStorage so the first render already reflects prior state.
  const [collapsedProjects, setCollapsedProjects] = useState<Set<string>>(() => {
    try {
      const raw = localStorage.getItem("bot-hq.cl.collapsedProjects");
      if (raw) return new Set(JSON.parse(raw) as string[]);
    } catch {
      // Bad JSON or localStorage disabled — fall through to empty.
    }
    return new Set();
  });
  useEffect(() => {
    try {
      localStorage.setItem(
        "bot-hq.cl.collapsedProjects",
        JSON.stringify([...collapsedProjects]),
      );
    } catch {
      // Storage quota or disabled — silent no-op.
    }
  }, [collapsedProjects]);

  const {
    data: entries = [],
    isLoading,
    refetch,
  } = useTauriQuery<ClIndexEntryView[]>("cl_index_search", {
    project,
    query: debouncedQuery.trim() || null,
  });

  // Project dropdown source — populated from `list_projects` so the filter is
  // discoverable instead of a free-text typo trap. Refetches every minute so
  // newly-imported projects appear without a manual page reload.
  const { data: projects = [] } = useTauriQuery<ProjectView[]>(
    "list_projects",
    {},
    { refetchInterval: 60_000 },
  );

  // File-content viewer: clicking a row opens a side pane with the file
  // content. State holds the {project, file_path} we're currently viewing
  // (null = pane closed). The actual fetch is a useTauriQuery so React
  // Query handles cache + loading + errors.
  const [selectedFile, setSelectedFile] = useState<{
    project: string;
    filePath: string;
  } | null>(null);
  const { data: fileContent, isFetching: fileLoading, error: fileError } =
    useTauriQuery<ClFileContentView>(
      "cl_read_file",
      selectedFile
        ? { project: selectedFile.project, filePath: selectedFile.filePath }
        : {},
      { enabled: selectedFile !== null },
    );

  // The currently-selected file's CL index entry — looked up locally from
  // the already-loaded `entries` rather than re-querying. Used by the inline
  // description editor in the file viewer pane (C6).
  const selectedEntry = useMemo(() => {
    if (!selectedFile) return null;
    return (
      entries.find(
        (e) =>
          e.project_id === selectedFile.project &&
          e.file_path === selectedFile.filePath,
      ) ?? null
    );
  }, [entries, selectedFile]);

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
        <select
          value={project ?? ""}
          onChange={(e) => setProject(e.target.value || null)}
          className={cn(
            "w-56 rounded-md border border-default bg-surface px-3 py-1.5 text-sm",
            "text-neutral-100 focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent",
          )}
          aria-label="Project filter"
        >
          <option value="">All projects</option>
          {projects.map((p) => (
            <option key={p.name} value={p.name}>
              {p.display_name || p.name}
            </option>
          ))}
        </select>
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
                    {files.map((f) => {
                      const isSelected =
                        selectedFile?.project === f.project_id &&
                        selectedFile?.filePath === f.file_path;
                      return (
                        <button
                          key={f.id}
                          type="button"
                          onClick={() =>
                            setSelectedFile({
                              project: f.project_id,
                              filePath: f.file_path,
                            })
                          }
                          className={cn(
                            "block w-full px-3 py-2 text-left transition-colors",
                            isSelected
                              ? "bg-accent/10 ring-1 ring-inset ring-accent/40"
                              : "hover:bg-elevated",
                          )}
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
                        </button>
                      );
                    })}
                  </div>
                )}
              </section>
            );
          })}
        </div>
      )}
      {selectedFile && (
        <div
          className="fixed inset-y-0 right-0 z-30 flex w-[min(720px,60vw)] flex-col border-l border-default bg-canvas shadow-2xl"
          role="dialog"
          aria-label={`File ${selectedFile.filePath}`}
        >
          <header className="flex items-center justify-between gap-3 border-b border-default px-4 py-2">
            <div className="min-w-0">
              <p className="truncate font-mono text-xs text-neutral-200">
                {selectedFile.filePath}
              </p>
              <p className="text-[0.65rem] text-neutral-500">
                {selectedFile.project}
                {fileContent && (
                  <>
                    <span className="mx-2 text-neutral-700">·</span>
                    {fileContent.size_bytes.toLocaleString()} bytes
                    {fileContent.truncated && (
                      <>
                        <span className="mx-2 text-neutral-700">·</span>
                        <span className="text-amber-400">
                          truncated to 1 MB
                        </span>
                      </>
                    )}
                  </>
                )}
              </p>
            </div>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setSelectedFile(null)}
              aria-label="Close file"
            >
              ×
            </Button>
          </header>
          <DescriptionEditor
            project={selectedFile.project}
            filePath={selectedFile.filePath}
            initial={selectedEntry?.description ?? ""}
            tags={selectedEntry?.tags ?? null}
            onSaved={() => refetch()}
          />
          <div className="min-h-0 flex-1 overflow-auto">
            {fileLoading && !fileContent ? (
              <div className="p-6 text-sm text-neutral-500">Loading…</div>
            ) : fileError ? (
              <div className="p-6 text-sm text-red-300">
                Failed to read: {String(fileError.message ?? fileError)}
              </div>
            ) : fileContent ? (
              <pre className="overflow-auto whitespace-pre-wrap px-4 py-3 font-mono text-xs text-neutral-200">
                {fileContent.content}
              </pre>
            ) : null}
          </div>
        </div>
      )}
    </div>
  );
}

// Inline description + tags editor for a CL index entry. Wraps the
// `cl_set_description` Tauri command (snake_case backend; tauri-specta
// camelCases the IPC keys). Saves are idempotent — backend treats an
// unknown (project, file_path) as upsert, so first-time descriptions on
// freshly-scanned files Just Work.
function DescriptionEditor({
  project,
  filePath,
  initial,
  tags,
  onSaved,
}: {
  project: string;
  filePath: string;
  initial: string;
  tags: string | null;
  onSaved: () => void;
}) {
  // Re-seed local drafts whenever the selected file changes — without this,
  // clicking a different file keeps the previous file's draft visible.
  const seedKey = `${project}/${filePath}`;
  const [seed, setSeed] = useState(seedKey);
  const [desc, setDesc] = useState(initial);
  const [tagsStr, setTagsStr] = useState(tags ?? "");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  if (seed !== seedKey) {
    setSeed(seedKey);
    setDesc(initial);
    setTagsStr(tags ?? "");
    setError(null);
  }

  const initialTagsStr = tags ?? "";
  const dirty = desc !== initial || tagsStr !== initialTagsStr;

  const handleSave = async () => {
    if (!dirty || saving) return;
    setSaving(true);
    setError(null);
    try {
      await invoke("cl_set_description", {
        project,
        filePath,
        description: desc,
        tags: tagsStr.trim() ? tagsStr.trim() : null,
      });
      onSaved();
    } catch (e) {
      const msg =
        e && typeof e === "object" && "message" in e
          ? String((e as { message: unknown }).message)
          : String(e);
      setError(msg);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="border-b border-default bg-surface px-4 py-3">
      <label className="block">
        <span className="mb-1 block text-[0.65rem] uppercase tracking-wide text-neutral-500">
          Description
        </span>
        <Textarea
          rows={2}
          value={desc}
          onChange={(e) => setDesc(e.target.value)}
          placeholder="One-line description shown in the CL index."
          className="w-full resize-y"
        />
      </label>
      <label className="mt-3 block">
        <span className="mb-1 block text-[0.65rem] uppercase tracking-wide text-neutral-500">
          Tags
        </span>
        <Input
          value={tagsStr}
          onChange={(e) => setTagsStr(e.target.value)}
          placeholder="(optional, comma-separated)"
          className="w-full"
        />
      </label>
      {error && (
        <p className="mt-2 text-xs text-red-300">
          Save failed: {error}{" "}
          <button
            onClick={() => setError(null)}
            className="underline hover:text-red-100"
          >
            dismiss
          </button>
        </p>
      )}
      <div className="mt-3 flex items-center justify-end gap-2">
        <Button
          variant="ghost"
          size="sm"
          disabled={!dirty || saving}
          onClick={() => {
            setDesc(initial);
            setTagsStr(initialTagsStr);
            setError(null);
          }}
        >
          Reset
        </Button>
        <Button
          variant="primary"
          size="sm"
          disabled={!dirty || saving}
          onClick={handleSave}
        >
          {saving ? "Saving…" : "Save"}
        </Button>
      </div>
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
