import { cn } from "../lib/cn";
import type {
  ClIndexEntryView,
  ClRescanReportView,
  ProjectView,
} from "../lib/bindings";
import {
  baseName,
  buildTree,
  collapseKey,
  type CtxTarget,
  FileIcon,
  FolderIcon,
  type OpenTab,
  PlusIcon,
  RefreshIcon,
  terminalInputClass,
  type TreeNode,
} from "./contextLibraryShared";
import { RescanIcon, WarnIcon, WrenchIcon } from "../components/icons";

const SIDEBAR_WIDTH = 240;
const INDENT_PX = 12;

// Icon-only header action. Color is applied per-button (cn is plain clsx —
// conflicting text-* classes would both stick).
const headerIconButtonClass = cn(
  "inline-flex items-center justify-center rounded p-1 transition-colors",
  "hover:bg-surface-container-high disabled:opacity-50",
  "disabled:hover:bg-transparent",
);

// ============================================================================
// WorkspaceSidebar (left ~240px) — "Library Tree"
// ============================================================================

interface WorkspaceSidebarProps {
  project: string | null;
  setProject: (v: string | null) => void;
  query: string;
  setQuery: (v: string) => void;
  projects: ProjectView[];
  byProject: Record<string, ClIndexEntryView[]>;
  byProjectFolders: Record<string, string[]>;
  isLoading: boolean;
  rescanning: boolean;
  rescanReport: ClRescanReportView | null;
  onRescan: () => void;
  collapsed: Set<string>;
  onToggle: (project: string, folderPath: string) => void;
  activeTab: OpenTab | null;
  onOpenFile: (project: string, filePath: string) => void;
  onOpenFolder: (project: string, folderPath: string) => void;
  onRequestRegister: () => void;
  onRequestMaintain: () => void;
  onContextMenu: (target: CtxTarget, x: number, y: number) => void;
}

export function WorkspaceSidebar({
  project,
  setProject,
  query,
  setQuery,
  projects,
  byProject,
  byProjectFolders,
  isLoading,
  rescanning,
  rescanReport,
  onRescan,
  collapsed,
  onToggle,
  activeTab,
  onOpenFile,
  onOpenFolder,
  onRequestRegister,
  onRequestMaintain,
  onContextMenu,
}: WorkspaceSidebarProps) {
  const projectIds = Object.keys(byProject).sort();
  const projectCount = projectIds.length;

  return (
    <aside
      className="flex h-full flex-shrink-0 flex-col border-r border-outline-variant bg-surface-container"
      style={{ width: SIDEBAR_WIDTH }}
    >
      <header className="flex items-center justify-between border-b border-outline-variant px-3 py-2">
        <span className="font-label-caps text-label-caps text-on-surface-variant">
          Library Tree
        </span>
        {/* New file/folder is created via right-click on a folder node (it
            needs the target folder + a name) — no header button for those. */}
        <div className="flex items-center gap-1">
          <button
            type="button"
            onClick={onRescan}
            disabled={rescanning || (!project && projectCount === 0)}
            aria-label={project ? `Rescan ${project}` : "Rescan all projects"}
            title={project ? `Rescan ${project}` : "Rescan all projects"}
            className={cn(
              headerIconButtonClass,
              "text-on-surface-variant hover:text-on-surface",
            )}
          >
            <RefreshIcon className={rescanning ? "animate-spin" : undefined} />
          </button>
          <button
            type="button"
            onClick={onRequestRegister}
            aria-label="Register project"
            title="Register an on-disk folder as a Context Library project"
            className={cn(
              headerIconButtonClass,
              "text-on-surface-variant hover:text-on-surface",
            )}
          >
            <PlusIcon className="size-3.5" />
          </button>
          <button
            type="button"
            onClick={onRequestMaintain}
            aria-label="Maintain CL"
            title="Dispatch a Brian + Rain session to maintain a project's Context Library"
            className={cn(headerIconButtonClass, "text-primary")}
          >
            <WrenchIcon size={14} />
          </button>
        </div>
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
        {rescanReport && (
          <div className="flex flex-wrap gap-2 rounded border border-outline-variant bg-surface-container-lowest px-2 py-1 font-code-sm text-code-sm">
            <span className="text-emerald-400">
              +{rescanReport.added.length}
            </span>
            <span className="inline-flex items-center gap-0.5 text-blue-400">
              <RescanIcon size={12} />
              {rescanReport.touched.length}
            </span>
            <span className="inline-flex items-center gap-0.5 text-amber-400">
              <WarnIcon size={12} />
              {rescanReport.orphaned.length}
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
          projectIds.map((projectId) => (
            <section key={projectId} className="mb-1">
              <FolderNode
                project={projectId}
                node={buildTree(
                  byProject[projectId],
                  byProjectFolders[projectId] ?? [],
                )}
                depth={0}
                isProjectRoot
                collapsed={collapsed}
                onToggle={onToggle}
                activeTab={activeTab}
                onOpenFile={onOpenFile}
                onOpenFolder={onOpenFolder}
                onContextMenu={onContextMenu}
              />
            </section>
          ))
        )}
      </div>
    </aside>
  );
}

// ============================================================================
// FolderNode — one folder row in the tree. A single click does BOTH things
// the user asked for: toggle collapse AND open the folder-view tab.
// ============================================================================

function countFiles(node: TreeNode): number {
  return (
    node.files.length + node.folders.reduce((s, f) => s + countFiles(f), 0)
  );
}

function FolderNode({
  project,
  node,
  depth,
  isProjectRoot = false,
  collapsed,
  onToggle,
  activeTab,
  onOpenFile,
  onOpenFolder,
  onContextMenu,
}: {
  project: string;
  node: TreeNode;
  depth: number;
  isProjectRoot?: boolean;
  collapsed: Set<string>;
  onToggle: (project: string, folderPath: string) => void;
  activeTab: OpenTab | null;
  onOpenFile: (project: string, filePath: string) => void;
  onOpenFolder: (project: string, folderPath: string) => void;
  onContextMenu: (target: CtxTarget, x: number, y: number) => void;
}) {
  const isCollapsed = collapsed.has(collapseKey(project, node.path));
  const isActive =
    activeTab?.kind === "folder" &&
    activeTab.project === project &&
    activeTab.folderPath === node.path;
  const label = isProjectRoot ? project : node.name;

  return (
    <div>
      <button
        type="button"
        onClick={() => {
          onToggle(project, node.path);
          onOpenFolder(project, node.path);
        }}
        onContextMenu={(e) => {
          e.preventDefault();
          onContextMenu(
            { project, path: node.path, kind: "folder" },
            e.clientX,
            e.clientY,
          );
        }}
        aria-expanded={!isCollapsed}
        title={node.path || project}
        style={{ paddingLeft: `${depth * INDENT_PX + 8}px` }}
        className={cn(
          "flex w-full items-center gap-1 rounded py-1 pr-2 text-left transition-colors",
          isProjectRoot
            ? "font-label-caps text-label-caps"
            : "font-code-sm text-code-sm",
          isActive
            ? "bg-primary/15 text-on-surface"
            : "text-on-surface-variant hover:bg-surface-container-high hover:text-on-surface",
        )}
      >
        <span
          aria-hidden
          className={cn(
            "inline-block w-3 shrink-0 text-on-surface-variant/60 transition-transform",
            isCollapsed ? "" : "rotate-90",
          )}
        >
          ▸
        </span>
        <FolderIcon className="shrink-0 text-on-surface-variant/60" />
        <span className="truncate">{label}</span>
        {isProjectRoot && (
          <span className="ml-auto pl-1 font-code-sm text-code-sm text-on-surface-variant/60">
            {countFiles(node)}
          </span>
        )}
      </button>
      {!isCollapsed && (
        <div>
          {node.folders.map((child) => (
            <FolderNode
              key={child.path}
              project={project}
              node={child}
              depth={depth + 1}
              collapsed={collapsed}
              onToggle={onToggle}
              activeTab={activeTab}
              onOpenFile={onOpenFile}
              onOpenFolder={onOpenFolder}
              onContextMenu={onContextMenu}
            />
          ))}
          {node.files.map((f) => (
            <FileRow
              key={f.id}
              file={f}
              depth={depth + 1}
              isActive={
                activeTab?.kind === "file" &&
                activeTab.project === f.project_id &&
                activeTab.filePath === f.file_path
              }
              onOpen={() => onOpenFile(f.project_id, f.file_path)}
              onContextMenu={onContextMenu}
            />
          ))}
        </div>
      )}
    </div>
  );
}

// ============================================================================
// FileRow — single clickable file entry in the tree
// ============================================================================

function FileRow({
  file,
  depth,
  isActive,
  onOpen,
  onContextMenu,
}: {
  file: ClIndexEntryView;
  depth: number;
  isActive: boolean;
  onOpen: () => void;
  onContextMenu: (target: CtxTarget, x: number, y: number) => void;
}) {
  return (
    <button
      type="button"
      onClick={onOpen}
      onContextMenu={(e) => {
        e.preventDefault();
        onContextMenu(
          { project: file.project_id, path: file.file_path, kind: "file" },
          e.clientX,
          e.clientY,
        );
      }}
      title={file.file_path}
      style={{ paddingLeft: `${depth * INDENT_PX + 8}px` }}
      className={cn(
        "flex w-full items-center gap-1 truncate border-l-2 py-1 pr-2 text-left font-code-sm text-code-sm transition-colors",
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
