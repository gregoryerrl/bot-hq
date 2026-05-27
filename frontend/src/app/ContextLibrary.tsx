import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTauriQuery } from "../hooks/useInvoke";
import { cn } from "../lib/cn";
import type {
  ClFileContentView,
  ClIndexEntryView,
  ClRescanReportView,
  ProjectView,
} from "../lib/bindings";

// ============================================================================
// ContextLibrary — 2-pane Industrial Terminal layout
//
//   ┌────────────┬──────────────────────────────────────────┐
//   │ WORKSPACE  │ [tabs] [×] [tabs] [×]   UNSAVED  [Save]  │
//   │  + search  │ ───────────────────────────────────────  │
//   │  + filter  │   1 │ # contents of the active tab       │
//   │  + rescan  │   2 │ ...                                │
//   │  ────────  │     │                                    │
//   │  tree:     │     │ description editor (working save)  │
//   │  ▾ proj    │                                          │
//   │    file ←  │                                          │
//   └────────────┴──────────────────────────────────────────┘
//
// File-content saves are disabled in v1 — `cl_write_file` doesn't exist.
// Description saves work via the existing `cl_set_description` command.
// ============================================================================

interface OpenTab {
  project: string;
  filePath: string;
}

const SIDEBAR_WIDTH = 240;

const terminalInputClass = cn(
  "w-full border-0 border-b border-outline-variant bg-transparent",
  "rounded-none px-0 py-1 font-code-sm text-code-sm text-on-surface",
  "placeholder:text-on-surface-variant caret-primary",
  "focus:border-primary focus:outline-none",
);

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

  const { data: projects = [] } = useTauriQuery<ProjectView[]>(
    "list_projects",
    {},
    { refetchInterval: 60_000 },
  );

  const byProject = useMemo(() => {
    const acc: Record<string, ClIndexEntryView[]> = {};
    for (const e of entries) {
      (acc[e.project_id] = acc[e.project_id] ?? []).push(e);
    }
    for (const k of Object.keys(acc)) {
      acc[k].sort((a, b) => a.file_path.localeCompare(b.file_path));
    }
    return acc;
  }, [entries]);

  // Multi-tab state. Opening a file that's already in `tabs` just focuses
  // its tab; otherwise a new tab is pushed and activated.
  const [tabs, setTabs] = useState<OpenTab[]>([]);
  const [activeTabIndex, setActiveTabIndex] = useState(0);
  const activeTab: OpenTab | null = tabs[activeTabIndex] ?? null;

  const openFile = (proj: string, filePath: string) => {
    const idx = tabs.findIndex(
      (t) => t.project === proj && t.filePath === filePath,
    );
    if (idx >= 0) {
      setActiveTabIndex(idx);
    } else {
      setTabs((prev) => {
        const next = [...prev, { project: proj, filePath }];
        // Activate the freshly-pushed tab. Index is the prev length.
        setActiveTabIndex(prev.length);
        return next;
      });
    }
  };

  const closeTab = (index: number) => {
    setTabs((prev) => {
      const next = prev.filter((_, i) => i !== index);
      setActiveTabIndex((current) => {
        if (next.length === 0) return 0;
        if (current === index) return Math.max(0, index - 1);
        if (current > index) return current - 1;
        return current;
      });
      return next;
    });
  };

  const handleRescan = async () => {
    if (rescanning) return;
    setRescanning(true);
    setRescanReport(null);
    try {
      if (project) {
        const report = await invoke<ClRescanReportView>("cl_rescan", {
          project,
        });
        setRescanReport(report);
      } else {
        // All-projects rescan: iterate over every project we know about
        // (derived from current results — same caveat as before).
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
    <div className="flex h-full bg-background">
      <WorkspaceSidebar
        project={project}
        setProject={setProject}
        query={query}
        setQuery={setQuery}
        projects={projects}
        byProject={byProject}
        isLoading={isLoading}
        rescanning={rescanning}
        rescanReport={rescanReport}
        onRescan={handleRescan}
        collapsedProjects={collapsedProjects}
        onToggleProject={toggleProject}
        activeTab={activeTab}
        onOpenFile={openFile}
      />
      <EditorArea
        tabs={tabs}
        activeTabIndex={activeTabIndex}
        onSelectTab={setActiveTabIndex}
        onCloseTab={closeTab}
        activeTab={activeTab}
        entries={entries}
        onRefetchIndex={refetch}
      />
    </div>
  );
}

// ============================================================================
// WorkspaceSidebar (left ~240px)
// ============================================================================

interface WorkspaceSidebarProps {
  project: string | null;
  setProject: (v: string | null) => void;
  query: string;
  setQuery: (v: string) => void;
  projects: ProjectView[];
  byProject: Record<string, ClIndexEntryView[]>;
  isLoading: boolean;
  rescanning: boolean;
  rescanReport: ClRescanReportView | null;
  onRescan: () => void;
  collapsedProjects: Set<string>;
  onToggleProject: (id: string) => void;
  activeTab: OpenTab | null;
  onOpenFile: (project: string, filePath: string) => void;
}

function WorkspaceSidebar({
  project,
  setProject,
  query,
  setQuery,
  projects,
  byProject,
  isLoading,
  rescanning,
  rescanReport,
  onRescan,
  collapsedProjects,
  onToggleProject,
  activeTab,
  onOpenFile,
}: WorkspaceSidebarProps) {
  const projectCount = Object.keys(byProject).length;

  return (
    <aside
      className="flex h-full flex-shrink-0 flex-col border-r border-outline-variant bg-surface-container"
      style={{ width: SIDEBAR_WIDTH }}
    >
      <header className="flex items-center justify-between border-b border-outline-variant px-3 py-2">
        <span className="font-label-caps text-label-caps text-on-surface-variant">
          WORKSPACE
        </span>
        <button
          type="button"
          disabled
          title="New file — backend not yet wired"
          aria-label="New file (disabled)"
          className="rounded p-1 text-on-surface-variant transition-colors hover:text-on-surface disabled:cursor-not-allowed disabled:opacity-40"
        >
          <PlusIcon />
        </button>
      </header>

      <div className="flex flex-col gap-2 border-b border-outline-variant px-3 py-2">
        <input
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search files…"
          className={terminalInputClass}
        />
        <select
          value={project ?? ""}
          onChange={(e) => setProject(e.target.value || null)}
          aria-label="Project filter"
          className="w-full rounded border border-outline-variant bg-surface-container-lowest px-2 py-1 font-code-sm text-code-sm text-on-surface focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
        >
          <option value="">All projects</option>
          {projects.map((p) => (
            <option key={p.name} value={p.name}>
              {p.display_name || p.name}
            </option>
          ))}
        </select>
        <button
          type="button"
          onClick={onRescan}
          disabled={rescanning || (!project && projectCount === 0)}
          title={
            project
              ? `cl_rescan(${project})`
              : `cl_rescan all (${projectCount} project${
                  projectCount === 1 ? "" : "s"
                })`
          }
          className="inline-flex items-center justify-center gap-1.5 rounded border border-outline-variant bg-surface-container-lowest px-2 py-1 font-code-sm text-code-sm text-on-surface transition-colors hover:bg-surface-container-high disabled:opacity-50"
        >
          <RefreshIcon />
          {rescanning
            ? "Rescanning…"
            : project
              ? "Rescan project"
              : `Rescan all (${projectCount})`}
        </button>
        {rescanReport && (
          <div className="flex flex-wrap gap-2 rounded border border-outline-variant bg-surface-container-lowest px-2 py-1 font-code-sm text-code-sm">
            <span className="text-emerald-400">
              +{rescanReport.added.length}
            </span>
            <span className="text-blue-400">
              ↻{rescanReport.touched.length}
            </span>
            <span className="text-amber-400">
              ⚠{rescanReport.orphaned.length}
            </span>
          </div>
        )}
      </div>

      <div className="flex-1 overflow-y-auto px-1 py-1">
        {isLoading ? (
          <div className="space-y-1 px-2 py-2">
            {[0, 1, 2].map((i) => (
              <div
                key={i}
                className="h-6 animate-pulse rounded bg-surface-container-high"
              />
            ))}
          </div>
        ) : projectCount === 0 ? (
          <p className="px-2 py-3 font-code-sm text-code-sm text-on-surface-variant">
            {query.trim() || project
              ? "No matches."
              : "Empty. Use Rescan to populate."}
          </p>
        ) : (
          Object.entries(byProject).map(([projectId, files]) => (
            <ProjectGroup
              key={projectId}
              projectId={projectId}
              files={files}
              collapsed={collapsedProjects.has(projectId)}
              onToggle={() => onToggleProject(projectId)}
              activeTab={activeTab}
              onOpenFile={onOpenFile}
            />
          ))
        )}
      </div>
    </aside>
  );
}

// ============================================================================
// ProjectGroup — one collapsible project section in the tree
// ============================================================================

function ProjectGroup({
  projectId,
  files,
  collapsed,
  onToggle,
  activeTab,
  onOpenFile,
}: {
  projectId: string;
  files: ClIndexEntryView[];
  collapsed: boolean;
  onToggle: () => void;
  activeTab: OpenTab | null;
  onOpenFile: (project: string, filePath: string) => void;
}) {
  return (
    <section className="mb-1">
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={!collapsed}
        className="flex w-full items-center gap-1 rounded px-2 py-1 text-left font-label-caps text-label-caps text-on-surface-variant transition-colors hover:bg-surface-container-high"
      >
        <span
          aria-hidden
          className={cn(
            "inline-block w-3 text-on-surface-variant/60 transition-transform",
            collapsed ? "" : "rotate-90",
          )}
        >
          ▸
        </span>
        <span className="truncate">{projectId}</span>
        <span className="ml-auto font-code-sm text-code-sm text-on-surface-variant/60">
          {files.length}
        </span>
      </button>
      {!collapsed && (
        <div>
          {files.map((f) => (
            <FileRow
              key={f.id}
              file={f}
              isActive={
                activeTab?.project === f.project_id &&
                activeTab?.filePath === f.file_path
              }
              onOpen={() => onOpenFile(f.project_id, f.file_path)}
            />
          ))}
        </div>
      )}
    </section>
  );
}

// ============================================================================
// FileRow — single clickable file entry in the tree
// ============================================================================

function FileRow({
  file,
  isActive,
  onOpen,
}: {
  file: ClIndexEntryView;
  isActive: boolean;
  onOpen: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onOpen}
      title={file.file_path}
      className={cn(
        "flex w-full items-center gap-1 truncate border-l-2 px-2 py-1 text-left font-code-sm text-code-sm transition-colors",
        isActive
          ? "border-primary bg-primary/15 text-on-surface"
          : "border-transparent text-on-surface-variant hover:bg-surface-container-high hover:text-on-surface",
      )}
    >
      <FileIcon className="shrink-0 text-on-surface-variant/60" />
      <span className="truncate">{baseName(file.file_path)}</span>
    </button>
  );
}

// ============================================================================
// EditorArea — tab strip + active tab content
// ============================================================================

interface EditorAreaProps {
  tabs: OpenTab[];
  activeTabIndex: number;
  onSelectTab: (i: number) => void;
  onCloseTab: (i: number) => void;
  activeTab: OpenTab | null;
  entries: ClIndexEntryView[];
  onRefetchIndex: () => void;
}

function EditorArea({
  tabs,
  activeTabIndex,
  onSelectTab,
  onCloseTab,
  activeTab,
  entries,
  onRefetchIndex,
}: EditorAreaProps) {
  return (
    <div className="flex min-w-0 flex-1 flex-col">
      {tabs.length > 0 && (
        <TabStrip
          tabs={tabs}
          activeTabIndex={activeTabIndex}
          onSelectTab={onSelectTab}
          onCloseTab={onCloseTab}
        />
      )}
      {activeTab ? (
        <EditorPane
          key={`${activeTab.project}/${activeTab.filePath}`}
          tab={activeTab}
          entries={entries}
          onRefetchIndex={onRefetchIndex}
        />
      ) : (
        <EmptyEditor />
      )}
    </div>
  );
}

// ============================================================================
// TabStrip — horizontal tab bar at the top of the editor area
// ============================================================================

function TabStrip({
  tabs,
  activeTabIndex,
  onSelectTab,
  onCloseTab,
}: {
  tabs: OpenTab[];
  activeTabIndex: number;
  onSelectTab: (i: number) => void;
  onCloseTab: (i: number) => void;
}) {
  return (
    <div className="flex flex-shrink-0 items-center overflow-x-auto border-b border-outline-variant bg-surface-container">
      {tabs.map((t, i) => {
        const active = i === activeTabIndex;
        return (
          <div
            key={`${t.project}/${t.filePath}`}
            className={cn(
              "group flex shrink-0 items-center gap-2 border-r border-outline-variant/40 px-3 py-2",
              "font-code-sm text-code-sm transition-colors",
              active
                ? "bg-background text-on-surface"
                : "text-on-surface-variant hover:bg-surface-container-high hover:text-on-surface",
            )}
          >
            <button
              type="button"
              onClick={() => onSelectTab(i)}
              title={`${t.project} — ${t.filePath}`}
              className="flex max-w-[200px] items-center gap-1.5"
            >
              <FileIcon className="shrink-0 text-on-surface-variant/60" />
              <span className="truncate">{baseName(t.filePath)}</span>
            </button>
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                onCloseTab(i);
              }}
              aria-label={`Close ${t.filePath}`}
              className="rounded p-0.5 text-on-surface-variant/60 transition-colors hover:bg-surface-container-highest hover:text-on-surface"
            >
              <CloseIcon />
            </button>
          </div>
        );
      })}
    </div>
  );
}

// ============================================================================
// EditorPane — file content + line-number gutter + description + footer
// ============================================================================

function EditorPane({
  tab,
  entries,
  onRefetchIndex,
}: {
  tab: OpenTab;
  entries: ClIndexEntryView[];
  onRefetchIndex: () => void;
}) {
  const { data: fileContent, isFetching, error: fileError } =
    useTauriQuery<ClFileContentView>(
      "cl_read_file",
      { project: tab.project, filePath: tab.filePath },
    );

  const entry = useMemo(
    () =>
      entries.find(
        (e) => e.project_id === tab.project && e.file_path === tab.filePath,
      ) ?? null,
    [entries, tab.project, tab.filePath],
  );

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex flex-shrink-0 items-center justify-between gap-3 border-b border-outline-variant bg-surface-container-low px-4 py-2">
        <div className="min-w-0">
          <p className="truncate font-code-sm text-code-sm text-on-surface">
            {tab.filePath}
          </p>
          <p className="font-code-sm text-code-sm text-on-surface-variant">
            {tab.project}
            {fileContent && (
              <>
                <span className="mx-2 text-on-surface-variant/60">·</span>
                {fileContent.size_bytes.toLocaleString()} bytes
                {fileContent.truncated && (
                  <>
                    <span className="mx-2 text-on-surface-variant/60">·</span>
                    <span className="text-amber-400">
                      truncated to 1 MB
                    </span>
                  </>
                )}
              </>
            )}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <span
            className="rounded border border-amber-500/40 bg-amber-500/15 px-2 py-0.5 font-label-caps text-label-caps text-amber-300 opacity-40"
            title="File-content edits aren't wired yet"
          >
            UNSAVED CHANGES
          </span>
          <button
            type="button"
            disabled
            title="Backend not yet wired — cl_write_file not implemented"
            className="inline-flex items-center gap-1.5 rounded border border-primary bg-primary px-3 py-1.5 font-code-sm text-code-sm text-on-primary disabled:cursor-not-allowed disabled:opacity-40"
          >
            <SaveIcon />
            Save Changes
          </button>
        </div>
      </header>

      <div className="min-h-0 flex-1 overflow-auto bg-surface-container-low">
        {isFetching && !fileContent ? (
          <p className="px-4 py-3 font-code-sm text-code-sm text-on-surface-variant">
            Loading…
          </p>
        ) : fileError ? (
          <p className="px-4 py-3 font-code-sm text-code-sm text-error">
            Failed to read: {String(fileError.message ?? fileError)}
          </p>
        ) : fileContent ? (
          <CodeView content={fileContent.content} />
        ) : null}
      </div>

      <DescriptionEditor
        project={tab.project}
        filePath={tab.filePath}
        initial={entry?.description ?? ""}
        tags={entry?.tags ?? null}
        onSaved={onRefetchIndex}
      />
    </div>
  );
}

// ============================================================================
// CodeView — content + line-number gutter (no syntax highlighting in v1)
// ============================================================================

function CodeView({ content }: { content: string }) {
  const lineCount = useMemo(
    () => Math.max(1, content.split("\n").length),
    [content],
  );
  const gutterWidthCh = String(lineCount).length + 1; // +1 for breathing room

  return (
    <div className="flex font-code-sm text-code-sm">
      <div
        className="select-none border-r border-outline-variant/30 px-3 py-3 text-right text-on-surface-variant/60"
        style={{ minWidth: `${gutterWidthCh}ch` }}
        aria-hidden
      >
        {Array.from({ length: lineCount }, (_, i) => (
          <div key={i} className="leading-relaxed">
            {i + 1}
          </div>
        ))}
      </div>
      <pre className="flex-1 overflow-x-auto whitespace-pre px-4 py-3 leading-relaxed text-on-surface">
        {content}
      </pre>
    </div>
  );
}

// ============================================================================
// EmptyEditor — shown when no tabs are open
// ============================================================================

function EmptyEditor() {
  return (
    <div className="flex min-h-0 flex-1 items-center justify-center bg-background">
      <div className="text-center">
        <p className="font-headline-md text-headline-md text-on-surface-variant">
          No file open
        </p>
        <p className="mt-1 font-code-sm text-code-sm text-on-surface-variant/60">
          Pick a file from the sidebar to open it in a tab.
        </p>
      </div>
    </div>
  );
}

// ============================================================================
// DescriptionEditor — kept from prior implementation, tokens updated.
//
// Wraps the `cl_set_description` Tauri command (snake_case backend;
// tauri-specta camelCases the IPC keys). Saves are idempotent — backend
// treats an unknown (project, file_path) as upsert.
// ============================================================================

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
    <div className="flex-shrink-0 border-t border-outline-variant bg-surface-container px-4 py-3">
      <label className="block">
        <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
          Description
        </span>
        <textarea
          rows={2}
          value={desc}
          onChange={(e) => setDesc(e.target.value)}
          placeholder="One-line description shown in the CL index."
          className="w-full resize-y rounded border border-outline-variant bg-surface-container-lowest px-2 py-1 font-code-sm text-code-sm text-on-surface placeholder:text-on-surface-variant focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
        />
      </label>
      <label className="mt-3 block">
        <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
          Tags
        </span>
        <input
          type="text"
          value={tagsStr}
          onChange={(e) => setTagsStr(e.target.value)}
          placeholder="(optional, comma-separated)"
          className={terminalInputClass}
        />
      </label>
      {error && (
        <p className="mt-2 font-code-sm text-code-sm text-error">
          Save failed: {error}{" "}
          <button
            onClick={() => setError(null)}
            className="underline hover:text-on-error-container"
          >
            dismiss
          </button>
        </p>
      )}
      <div className="mt-3 flex items-center justify-end gap-2">
        <button
          type="button"
          disabled={!dirty || saving}
          onClick={() => {
            setDesc(initial);
            setTagsStr(initialTagsStr);
            setError(null);
          }}
          className="rounded border border-outline-variant bg-transparent px-3 py-1 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:bg-surface-container-high hover:text-on-surface disabled:opacity-50"
        >
          Reset
        </button>
        <button
          type="button"
          disabled={!dirty || saving}
          onClick={handleSave}
          className="rounded border border-primary bg-primary px-3 py-1 font-code-sm text-code-sm text-on-primary transition-colors hover:bg-primary-fixed disabled:opacity-50"
        >
          {saving ? "Saving…" : "Save Description"}
        </button>
      </div>
    </div>
  );
}

// ============================================================================
// helpers
// ============================================================================

function baseName(filePath: string): string {
  const parts = filePath.split("/");
  return parts[parts.length - 1] || filePath;
}

// ============================================================================
// Inline SVG icons
// ============================================================================

function PlusIcon({ className }: { className?: string }) {
  return (
    <svg
      className={cn("size-4", className)}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <line x1="12" y1="5" x2="12" y2="19" />
      <line x1="5" y1="12" x2="19" y2="12" />
    </svg>
  );
}

function RefreshIcon({ className }: { className?: string }) {
  return (
    <svg
      className={cn("size-3.5", className)}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <polyline points="23 4 23 10 17 10" />
      <path d="M20.49 15a9 9 0 11-2.12-9.36L23 10" />
    </svg>
  );
}

function FileIcon({ className }: { className?: string }) {
  return (
    <svg
      className={cn("size-3.5", className)}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" />
      <polyline points="14 2 14 8 20 8" />
    </svg>
  );
}

function CloseIcon({ className }: { className?: string }) {
  return (
    <svg
      className={cn("size-3.5", className)}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <line x1="18" y1="6" x2="6" y2="18" />
      <line x1="6" y1="6" x2="18" y2="18" />
    </svg>
  );
}

function SaveIcon({ className }: { className?: string }) {
  return (
    <svg
      className={cn("size-3.5", className)}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <path d="M19 21H5a2 2 0 01-2-2V5a2 2 0 012-2h11l5 5v11a2 2 0 01-2 2z" />
      <polyline points="17 21 17 13 7 13 7 21" />
      <polyline points="7 3 7 8 15 8" />
    </svg>
  );
}
