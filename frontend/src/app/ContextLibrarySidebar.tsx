import { memo, useMemo, type ReactNode } from "react";
import { cn } from "../lib/cn";
import type {
  ClIndexEntryView,
  ClRescanReportView,
  ProjectView,
} from "../lib/bindings";
import {
  baseName,
  buildTree,
  CollapseAllIcon,
  collapseKey,
  type CtxTarget,
  FileIcon,
  FolderIcon,
  type OpenTab,
  PlusIcon,
  RefreshIcon,
  splitGlobals,
  terminalInputClass,
  treeProjectIds,
  type TreeNode,
} from "./contextLibraryShared";
import { RescanIcon, WarnIcon, WrenchIcon } from "../components/icons";

const INDENT_PX = 12;

// Shared read-only empty tree for a registered-but-empty project (no files, no
// described folders) — lets the memoized tree map fall back without an inline
// buildTree([], []) per such node. Never mutated (nodes only read it).
const EMPTY_TREE: TreeNode = { name: "", path: "", folders: [], files: [] };

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
  /** Pixel width, owned by ContextLibrary's drag-resize state. */
  width: number;
  query: string;
  setQuery: (v: string) => void;
  projects: ProjectView[];
  byProject: Record<string, ClIndexEntryView[]>;
  byProjectFolders: Record<string, string[]>;
  isLoading: boolean;
  rescanning: boolean;
  rescanReport: ClRescanReportView | null;
  rescanFailures: string[];
  onRescan: () => void;
  onCollapseAll: () => void;
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
  width,
  query,
  setQuery,
  projects,
  byProject,
  byProjectFolders,
  isLoading,
  rescanning,
  rescanReport,
  rescanFailures,
  onRescan,
  onCollapseAll,
  collapsed,
  onToggle,
  activeTab,
  onOpenFile,
  onOpenFolder,
  onRequestRegister,
  onRequestMaintain,
  onContextMenu,
}: WorkspaceSidebarProps) {
  const projectIds = treeProjectIds(
    Object.keys(byProject),
    projects.map((p) => p.name),
    query.trim() !== "",
    null,
  );
  const projectCount = projectIds.length;

  // Category split: PROJECTS = registered projects; the `_globals` bucket is
  // divided into SYSTEM (bot-hq-owned, read+update only) and GLOBAL (loose
  // cross-project files). Buckets that are neither registered nor `_globals`
  // shouldn't occur (rescan upserts a project row) — they land in GLOBAL as a
  // defensive catch-all.
  const registered = new Set(projects.map((p) => p.name));
  // Memoize the O(files) tree builds on the index data (O6): search typing,
  // sidebar drag-resize, and collapse toggles re-render this component but DON'T
  // change byProject/byProjectFolders, so the trees are reused instead of every
  // project's tree being rebuilt + re-sorted each render.
  const { globalsSplit, globalTree, systemTree, projectTrees } = useMemo(() => {
    const split = splitGlobals(
      byProject["_globals"] ?? [],
      byProjectFolders["_globals"] ?? [],
    );
    const trees: Record<string, TreeNode> = {};
    const ids = new Set([
      ...Object.keys(byProject),
      ...Object.keys(byProjectFolders),
    ]);
    for (const id of ids) {
      if (id === "_globals") continue;
      trees[id] = buildTree(byProject[id] ?? [], byProjectFolders[id] ?? []);
    }
    return {
      globalsSplit: split,
      globalTree: buildTree(split.global.entries, split.global.folderPaths),
      systemTree: buildTree(split.system.entries, split.system.folderPaths),
      projectTrees: trees,
    };
  }, [byProject, byProjectFolders]);
  const projectCategoryIds = projectIds.filter(
    (id) => id !== "_globals" && registered.has(id),
  );
  const orphanIds = projectIds.filter(
    (id) => id !== "_globals" && !registered.has(id),
  );

  // With a search active, hide categories that have nothing to show;
  // otherwise all three headers always render (GLOBAL must stay right-click
  // reachable even when empty).
  const filterActive = query.trim() !== "";
  const hasProjects = projectCategoryIds.length > 0;
  const hasGlobal =
    globalsSplit.global.entries.length > 0 ||
    globalsSplit.global.folderPaths.length > 0 ||
    orphanIds.length > 0;
  const hasSystem =
    globalsSplit.system.entries.length > 0 ||
    globalsSplit.system.folderPaths.length > 0;
  const showProjects = !filterActive || hasProjects;
  const showGlobal = !filterActive || hasGlobal;
  const showSystem = !filterActive || hasSystem;

  const projectsFileCount = projectCategoryIds.reduce(
    (s, id) => s + (byProject[id]?.length ?? 0),
    0,
  );
  const globalFileCount =
    globalsSplit.global.entries.length +
    orphanIds.reduce((s, id) => s + (byProject[id]?.length ?? 0), 0);

  const nodeProps = {
    collapsed,
    onToggle,
    activeTab,
    onOpenFile,
    onOpenFolder,
    onContextMenu,
  };

  return (
    <aside
      className="flex h-full flex-shrink-0 flex-col border-r border-outline-variant bg-surface-container"
      style={{ width }}
    >
      {/* No "Library Tree" label here — the subtab pill above IS the header.
          Proposals/measurement moved to the Context Manager subtab. */}
      <header className="flex items-center border-b border-outline-variant px-3 py-2">
        {/* New file/folder is created via right-click on a folder node (it
            needs the target folder + a name) — no header button for those. */}
        <div className="flex items-center gap-1">
          <button
            type="button"
            onClick={onCollapseAll}
            disabled={projectCount === 0}
            aria-label="Collapse all folders"
            title="Collapse all folders"
            className={cn(
              headerIconButtonClass,
              "text-on-surface-variant hover:text-on-surface",
            )}
          >
            <CollapseAllIcon />
          </button>
          <button
            type="button"
            onClick={onRescan}
            disabled={rescanning || projectCount === 0}
            aria-label="Rescan all projects"
            title="Rescan all projects"
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
        {(rescanReport || rescanFailures.length > 0) && (
          <div className="flex flex-wrap gap-2 rounded border border-outline-variant bg-surface-container-lowest px-2 py-1 font-code-sm text-code-sm">
            {rescanReport && (
              <>
                <span className="text-success">
                  +{rescanReport.added.length}
                </span>
                <span className="inline-flex items-center gap-0.5 text-tertiary">
                  <RescanIcon size={12} />
                  {rescanReport.touched.length}
                </span>
                <span className="inline-flex items-center gap-0.5 text-warning">
                  <WarnIcon size={12} />
                  {rescanReport.orphaned.length}
                </span>
              </>
            )}
            {rescanFailures.length > 0 && (
              <span
                className="text-error"
                title={`Rescan failed for: ${rescanFailures.join(", ")}`}
              >
                ✗{rescanFailures.length} failed
              </span>
            )}
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
            {query.trim() ? "No matches." : "Empty. Use Rescan to populate."}
          </p>
        ) : (
          <>
            {showProjects && (
              <CategorySection
                id="@cat:projects"
                label="Projects"
                colorClass="text-primary"
                count={projectsFileCount}
                collapsed={collapsed}
                onToggle={onToggle}
              >
                {projectCategoryIds.map((projectId) => (
                  <FolderNode
                    key={projectId}
                    project={projectId}
                    node={projectTrees[projectId] ?? EMPTY_TREE}
                    depth={1}
                    isProjectRoot
                    {...nodeProps}
                  />
                ))}
              </CategorySection>
            )}
            {showGlobal && (
              <CategorySection
                id="@cat:global"
                label="Global"
                colorClass="text-on-surface-variant"
                count={globalFileCount}
                collapsed={collapsed}
                onToggle={onToggle}
                onContextMenu={(x, y) =>
                  onContextMenu(
                    { project: "_globals", path: "", kind: "folder" },
                    x,
                    y,
                  )
                }
              >
                {globalTree.folders.map((child) => (
                  <FolderNode
                    key={child.path}
                    project="_globals"
                    node={child}
                    depth={1}
                    {...nodeProps}
                  />
                ))}
                {globalTree.files.map((f) => (
                  <FileRow
                    key={f.id}
                    file={f}
                    depth={1}
                    isActive={
                      activeTab?.kind === "file" &&
                      activeTab.project === f.project_id &&
                      activeTab.filePath === f.file_path
                    }
                    onOpenFile={onOpenFile}
                    onContextMenu={onContextMenu}
                  />
                ))}
                {orphanIds.map((projectId) => (
                  <FolderNode
                    key={projectId}
                    project={projectId}
                    node={projectTrees[projectId] ?? EMPTY_TREE}
                    depth={1}
                    isProjectRoot
                    {...nodeProps}
                  />
                ))}
              </CategorySection>
            )}
            {showSystem && (
              <CategorySection
                id="@cat:system"
                label="System"
                colorClass="text-warning"
                count={globalsSplit.system.entries.length}
                collapsed={collapsed}
                onToggle={onToggle}
              >
                {systemTree.folders.map((child) => (
                  <FolderNode
                    key={child.path}
                    project="_globals"
                    node={child}
                    depth={1}
                    {...nodeProps}
                  />
                ))}
                {systemTree.files.map((f) => (
                  <FileRow
                    key={f.id}
                    file={f}
                    depth={1}
                    isActive={
                      activeTab?.kind === "file" &&
                      activeTab.project === f.project_id &&
                      activeTab.filePath === f.file_path
                    }
                    onOpenFile={onOpenFile}
                    onContextMenu={onContextMenu}
                  />
                ))}
              </CategorySection>
            )}
          </>
        )}
      </div>
    </aside>
  );
}

// ============================================================================
// CategorySection — a collapsible top-level grouping (Projects / Global /
// System). Left-click ONLY toggles collapse — categories never open a tab.
// Collapse state reuses the persisted `collapsed` set via sentinel project
// ids ("@cat:…"), which can't collide with real registered project names.
// ============================================================================

function CategorySection({
  id,
  label,
  colorClass,
  count,
  collapsed,
  onToggle,
  onContextMenu,
  children,
}: {
  id: string;
  label: string;
  colorClass: string;
  count: number;
  collapsed: Set<string>;
  onToggle: (project: string, folderPath: string) => void;
  onContextMenu?: (x: number, y: number) => void;
  children: ReactNode;
}) {
  const isCollapsed = collapsed.has(collapseKey(id, ""));
  return (
    <section className="mb-1">
      <button
        type="button"
        onClick={() => onToggle(id, "")}
        onContextMenu={
          onContextMenu
            ? (e) => {
                e.preventDefault();
                onContextMenu(e.clientX, e.clientY);
              }
            : undefined
        }
        aria-expanded={!isCollapsed}
        className={cn(
          "flex w-full items-center gap-1 rounded py-1 pl-2 pr-2 text-left font-label-caps text-label-caps transition-colors hover:bg-surface-container-high",
          colorClass,
        )}
      >
        <span
          aria-hidden
          className={cn(
            "inline-block w-3 shrink-0 transition-transform",
            isCollapsed ? "" : "rotate-90",
          )}
        >
          ▸
        </span>
        <span className="truncate">{label}</span>
        <span className="ml-auto pl-1 font-code-sm text-code-sm text-on-surface-variant/60">
          {count}
        </span>
      </button>
      {!isCollapsed && <div>{children}</div>}
    </section>
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

// Memoized (O6): a node skips re-render when its props (the memoized `node`,
// `collapsed`, `activeTab`, and the stabilized callbacks) are unchanged — so
// search typing / drag-resize don't re-render every node. Recursive children
// render via the memoized `FolderNode` const (function hoisting makes the impl
// available to `memo()` above its declaration).
const FolderNode = memo(FolderNodeImpl);

function FolderNodeImpl({
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
              onOpenFile={onOpenFile}
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

const FileRow = memo(FileRowImpl);

function FileRowImpl({
  file,
  depth,
  isActive,
  onOpenFile,
  onContextMenu,
}: {
  file: ClIndexEntryView;
  depth: number;
  isActive: boolean;
  onOpenFile: (project: string, filePath: string) => void;
  onContextMenu: (target: CtxTarget, x: number, y: number) => void;
}) {
  return (
    <button
      type="button"
      onClick={() => onOpenFile(file.project_id, file.file_path)}
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
