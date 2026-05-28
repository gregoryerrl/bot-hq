import { cn } from "../lib/cn";
import type {
  ClIndexEntryView,
  ClRescanReportView,
  ProjectView,
} from "../lib/bindings";
import {
  baseName,
  FileIcon,
  type OpenTab,
  PlusIcon,
  RefreshIcon,
  terminalInputClass,
} from "./contextLibraryShared";

const SIDEBAR_WIDTH = 240;

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

export function WorkspaceSidebar({
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
