import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTauriQuery, errorMessage } from "../hooks/useInvoke";
import type {
  ClFolderView,
  ClIndexEntryView,
  ClRescanReportView,
  ProjectView,
} from "../lib/bindings";
import {
  baseName,
  collapseKey,
  type CtxTarget,
  isInternalGlobalsPath,
  tabKey,
  type OpenTab,
} from "./contextLibraryShared";
import { WorkspaceSidebar } from "./ContextLibrarySidebar";
import { EditorArea } from "./ContextLibraryEditor";
import { RegisterProjectModal } from "./ContextLibraryRegisterModal";
import { MaintainCLModal } from "./MaintainCLModal";
import {
  ActionModal,
  ContextMenu,
  type ContextMenuItem,
} from "./ContextLibraryContextMenu";

// ============================================================================
// ContextLibrary — 2-pane Industrial Terminal layout. The left WorkspaceSidebar
// and the right EditorArea live in sibling files; shared types/helpers/icons in
// contextLibraryShared.
//
//   ┌────────────┬──────────────────────────────────────────┐
//   │ WORKSPACE  │ [tabs] [×] [tabs] [×]   UNSAVED  [Save]  │
//   │  + search  │ ───────────────────────────────────────  │
//   │  + filter  │   1 │ # contents of the active tab       │
//   │  + rescan  │   2 │ ...                                │
//   │  ────────  │     │                                    │
//   │  tree:     │     │ description editor (working save)  │
//   │  ▾ proj    │     │                                    │
//   │    file ←  │     │                                    │
//   └────────────┴──────────────────────────────────────────┘
//
// File-content saves go through `cl_write_file` (wired in ContextLibraryEditor);
// description saves via the existing `cl_set_description` command.
// ============================================================================

type CtxAction =
  | { mode: "newFile"; target: CtxTarget }
  | { mode: "newFolder"; target: CtxTarget }
  | { mode: "rename"; target: CtxTarget }
  | { mode: "delete"; target: CtxTarget }
  | { mode: "registerProject"; target: CtxTarget };

// Drag-resize bounds for the workspace sidebar (VS-Code-style explorer).
const SIDEBAR_MIN_PX = 180;
const SIDEBAR_MAX_PX = 480;
const SIDEBAR_DEFAULT_PX = 240;

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

  // Resizable sidebar, same listener pattern as SessionView's chat/document
  // split but in absolute px (explorer-style). Seeded from localStorage and
  // clamped so the tree can't collapse away or swallow the editor.
  const containerRef = useRef<HTMLDivElement>(null);
  const [sidebarWidth, setSidebarWidth] = useState<number>(() => {
    const saved = Number(localStorage.getItem("bot-hq.cl.sidebarWidth"));
    return Number.isFinite(saved) &&
      saved >= SIDEBAR_MIN_PX &&
      saved <= SIDEBAR_MAX_PX
      ? saved
      : SIDEBAR_DEFAULT_PX;
  });
  const onSidebarHandleDown = (e: React.MouseEvent) => {
    e.preventDefault();
    const container = containerRef.current;
    if (!container) return;
    let latest = sidebarWidth;
    const onMove = (ev: MouseEvent) => {
      const rect = container.getBoundingClientRect();
      latest = Math.min(
        SIDEBAR_MAX_PX,
        Math.max(SIDEBAR_MIN_PX, ev.clientX - rect.left),
      );
      setSidebarWidth(latest);
    };
    const onUp = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      localStorage.setItem("bot-hq.cl.sidebarWidth", String(Math.round(latest)));
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
  };

  const [rescanning, setRescanning] = useState(false);
  const [rescanReport, setRescanReport] = useState<ClRescanReportView | null>(
    null,
  );
  // Projects whose rescan FAILED — rendered beside the report chip so a bad
  // project can't hide inside a clean-looking aggregate.
  const [rescanFailures, setRescanFailures] = useState<string[]>([]);
  const [registerOpen, setRegisterOpen] = useState(false);
  const [maintainOpen, setMaintainOpen] = useState(false);

  // Persist expand/collapse choices across route navigation + restarts. Keyed
  // by collapseKey(project, folderPath) — the project-root node uses "".
  const [collapsed, setCollapsed] = useState<Set<string>>(() => {
    try {
      const raw = localStorage.getItem("bot-hq.cl.collapsed");
      if (raw) return new Set(JSON.parse(raw) as string[]);
    } catch {
      // Bad JSON or localStorage disabled — fall through to empty.
    }
    return new Set();
  });
  useEffect(() => {
    try {
      localStorage.setItem("bot-hq.cl.collapsed", JSON.stringify([...collapsed]));
    } catch {
      // Storage quota or disabled — silent no-op.
    }
  }, [collapsed]);

  const {
    data: entries = [],
    isLoading,
    refetch,
  } = useTauriQuery<ClIndexEntryView[]>("cl_index_search", {
    project,
    query: debouncedQuery.trim() || null,
  });

  const { data: projects = [], refetch: refetchProjects } = useTauriQuery<
    ProjectView[]
  >("list_projects", {}, { refetchInterval: 60_000 });

  // Folder descriptions feed both the tree (so a described-but-empty folder
  // still shows) and the folder-view editor (current description lookup).
  const { data: folders = [], refetch: refetchFolders } = useTauriQuery<
    ClFolderView[]
  >("cl_folder_search", {
    project,
    query: debouncedQuery.trim() || null,
  });

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

  const byProjectFolders = useMemo(() => {
    const acc: Record<string, string[]> = {};
    for (const f of folders) {
      (acc[f.project_id] = acc[f.project_id] ?? []).push(f.folder_path);
    }
    return acc;
  }, [folders]);

  // After register/unregister: refresh projects + index + folders so the tree,
  // the project filter, and any open folder-view all reflect the change.
  const onProjectChanged = () => {
    refetchProjects();
    refetch();
    refetchFolders();
  };

  // Right-click context menu + the new-file / rename / delete action modal.
  const [menu, setMenu] = useState<{
    target: CtxTarget;
    x: number;
    y: number;
  } | null>(null);
  const [action, setAction] = useState<CtxAction | null>(null);
  const [actionBusy, setActionBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  const menuItems = (target: CtxTarget): ContextMenuItem[] => {
    // SYSTEM subtree (agent custom-instructions, custom-general-rules.md) is
    // read+update only — no create/rename/delete. The backend guard in
    // cl_rename/cl_delete_path enforces the same; this hides the menu.
    if (target.project === "_globals" && isInternalGlobalsPath(target.path)) {
      return [];
    }
    if (target.kind === "folder") {
      const items: ContextMenuItem[] = [
        {
          label: "New file",
          onSelect: () => setAction({ mode: "newFile", target }),
        },
        {
          label: "New folder",
          onSelect: () => setAction({ mode: "newFolder", target }),
        },
      ];
      // Top-level folder in GLOBAL → promote to a registered project (moves
      // the folder under projects/).
      if (
        target.project === "_globals" &&
        target.path !== "" &&
        !target.path.includes("/")
      ) {
        items.push({
          label: "Register as project",
          onSelect: () => setAction({ mode: "registerProject", target }),
        });
      }
      // The project-root folder (and the Global category header, which
      // targets the `_globals` root) can't be renamed/deleted from here.
      if (target.path !== "") {
        items.push({
          label: "Rename",
          onSelect: () => setAction({ mode: "rename", target }),
        });
        items.push({
          label: "Delete",
          danger: true,
          onSelect: () => setAction({ mode: "delete", target }),
        });
      }
      return items;
    }
    return [
      {
        label: "Rename",
        onSelect: () => setAction({ mode: "rename", target }),
      },
      {
        label: "Delete",
        danger: true,
        onSelect: () => setAction({ mode: "delete", target }),
      },
    ];
  };

  const actionModalConfig = (a: CtxAction) => {
    const t = a.target;
    switch (a.mode) {
      case "newFile":
        return {
          title: "New file",
          inputLabel: "File name",
          confirmLabel: "Create",
        };
      case "newFolder":
        return {
          title: "New folder",
          inputLabel: "Folder name",
          confirmLabel: "Create",
        };
      case "rename":
        return {
          title: "Rename",
          inputLabel: "New name",
          initialValue: baseName(t.path),
          confirmLabel: "Rename",
        };
      case "delete":
        return {
          title: `Delete ${t.kind}`,
          message: `Delete "${baseName(t.path) || t.project}"? This permanently removes it from disk and cannot be undone.`,
          confirmLabel: "Delete",
          danger: true,
        };
      case "registerProject":
        return {
          title: "Register as project",
          message: `Moves "${t.path}" into projects/ and registers it as a Context Library project.`,
          inputLabel: "Project name",
          initialValue: baseName(t.path),
          confirmLabel: "Register",
        };
    }
  };

  const runAction = async (value: string) => {
    if (!action) return;
    const { target, mode } = action;
    setActionBusy(true);
    setActionError(null);
    try {
      if (mode === "registerProject") {
        const name = value;
        if (name === "_globals" || name === "projects" || name.includes("/")) {
          setActionError(`"${name}" is a reserved or invalid project name.`);
          return;
        }
        if (projects.some((p) => p.name === name)) {
          setActionError(`A project named "${name}" already exists.`);
          return;
        }
        // Physically move the folder under projects/ — registering it in
        // place would double-index it (the `_globals` walk only skips the
        // projects/ dir itself, not arbitrary registered roots).
        await invoke("cl_rename", {
          project: "_globals",
          fromPath: target.path,
          toPath: `projects/${name}`,
        });
        try {
          await invoke("cl_register_project", {
            name,
            displayName: name,
            workingRepoPath: null,
            clPath: null,
            description: null,
          });
        } catch (e) {
          // Roll the move back so the folder doesn't vanish from the tree
          // (the `_globals` walk skips everything under projects/).
          await invoke("cl_rename", {
            project: "_globals",
            fromPath: `projects/${name}`,
            toPath: target.path,
          }).catch(() => undefined);
          throw e;
        }
        // File rows re-home on rescan (orphans auto-purge), but
        // folder-description rows don't — re-point them at the new project.
        // Fetch the FULL `_globals` folder set fresh: the view's `folders`
        // query is filtered by the sidebar's project/search state, so
        // iterating it here could silently skip descendants.
        const allGlobals = await invoke<ClFolderView[]>("cl_folder_search", {
          project: "_globals",
          query: null,
        });
        const prefix = `${target.path}/`;
        const failedDescriptions: string[] = [];
        for (const f of allGlobals) {
          if (f.folder_path !== target.path && !f.folder_path.startsWith(prefix))
            continue;
          const newPath =
            f.folder_path === target.path
              ? ""
              : f.folder_path.slice(prefix.length);
          try {
            await invoke("cl_set_folder_description", {
              project: name,
              folderPath: newPath,
              description: f.description,
              tags: f.tags,
            });
            await invoke("cl_delete_folder_description", {
              project: "_globals",
              folderPath: f.folder_path,
            });
          } catch {
            failedDescriptions.push(f.folder_path || name);
          }
        }
        await invoke("cl_rescan", { project: name });
        if (failedDescriptions.length > 0) {
          // Registration itself succeeded — surface the partial description
          // migration instead of closing as if it were clean. The rows can be
          // re-added in the folder view.
          await invoke("cl_rescan", { project: target.project });
          onProjectChanged();
          setActionError(
            `Registered, but ${failedDescriptions.length} folder description(s) did not migrate: ${failedDescriptions.join(", ")}`,
          );
          return;
        }
      } else if (mode === "newFile") {
        const fp = target.path ? `${target.path}/${value}` : value;
        await invoke("cl_create_file", { project: target.project, filePath: fp });
      } else if (mode === "newFolder") {
        const fp = target.path ? `${target.path}/${value}` : value;
        await invoke("cl_mkdir", {
          project: target.project,
          folderPath: fp,
        });
      } else if (mode === "rename") {
        const slash = target.path.lastIndexOf("/");
        const parent = slash >= 0 ? target.path.slice(0, slash) : "";
        const to = parent ? `${parent}/${value}` : value;
        await invoke("cl_rename", {
          project: target.project,
          fromPath: target.path,
          toPath: to,
        });
      } else {
        await invoke("cl_delete_path", {
          project: target.project,
          path: target.path,
        });
        if (target.kind === "folder") {
          // Best-effort: drop the deleted folder's own description row.
          try {
            await invoke("cl_delete_folder_description", {
              project: target.project,
              folderPath: target.path,
            });
          } catch {
            // non-fatal
          }
        }
      }
      await invoke("cl_rescan", { project: target.project });
      onProjectChanged();
      setAction(null);
    } catch (e) {
      setActionError(errorMessage(e));
    } finally {
      setActionBusy(false);
    }
  };

  // Multi-tab state. Opening a file that's already in `tabs` just focuses
  // its tab; otherwise a new tab is pushed and activated.
  const [tabs, setTabs] = useState<OpenTab[]>([]);
  const [activeTabIndex, setActiveTabIndex] = useState(0);
  const activeTab: OpenTab | null = tabs[activeTabIndex] ?? null;

  const openTab = (tab: OpenTab) => {
    const key = tabKey(tab);
    const idx = tabs.findIndex((t) => tabKey(t) === key);
    if (idx >= 0) {
      setActiveTabIndex(idx);
    } else {
      setTabs((prev) => {
        const next = [...prev, tab];
        // Activate the freshly-pushed tab. Index is the prev length.
        setActiveTabIndex(prev.length);
        return next;
      });
    }
  };
  const openFile = (project: string, filePath: string) =>
    openTab({ kind: "file", project, filePath });
  const openFolder = (project: string, folderPath: string) =>
    openTab({ kind: "folder", project, folderPath });

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
    setRescanFailures([]);
    try {
      if (project) {
        try {
          const report = await invoke<ClRescanReportView>("cl_rescan", {
            project,
          });
          setRescanReport(report);
        } catch (e) {
          // eslint-disable-next-line no-console
          console.warn(`cl_rescan(${project}) failed`, e);
          setRescanFailures([project]);
        }
      } else {
        // All-projects rescan: each project's rescan is independent, so run
        // them in parallel (was a serial for…await). Per-project failures are
        // contained so one bad project doesn't abort the rest — but they're
        // collected and surfaced, not swallowed into a clean aggregate.
        const projectIds = Object.keys(byProject);
        const failures: string[] = [];
        const reports = await Promise.all(
          projectIds.map((p) =>
            invoke<ClRescanReportView>("cl_rescan", { project: p }).catch(
              (e) => {
                // eslint-disable-next-line no-console
                console.warn(`cl_rescan(${p}) failed`, e);
                failures.push(p);
                return null;
              },
            ),
          ),
        );
        setRescanFailures(failures);
        const agg: ClRescanReportView = { added: [], touched: [], orphaned: [] };
        for (const r of reports) {
          if (!r) continue;
          agg.added.push(...r.added);
          agg.touched.push(...r.touched);
          agg.orphaned.push(...r.orphaned);
        }
        setRescanReport(agg);
      }
      refetch();
      refetchFolders();
    } finally {
      setRescanning(false);
    }
  };

  const toggle = (project: string, folderPath: string) => {
    const key = collapseKey(project, folderPath);
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  return (
    <div ref={containerRef} className="flex h-full bg-background">
      <WorkspaceSidebar
        width={sidebarWidth}
        project={project}
        setProject={setProject}
        query={query}
        setQuery={setQuery}
        projects={projects}
        byProject={byProject}
        byProjectFolders={byProjectFolders}
        isLoading={isLoading}
        rescanning={rescanning}
        rescanReport={rescanReport}
        rescanFailures={rescanFailures}
        onRescan={handleRescan}
        collapsed={collapsed}
        onToggle={toggle}
        activeTab={activeTab}
        onOpenFile={openFile}
        onOpenFolder={openFolder}
        onRequestRegister={() => setRegisterOpen(true)}
        onRequestMaintain={() => setMaintainOpen(true)}
        onContextMenu={(target, x, y) => {
          // SYSTEM nodes have no actions at all — show nothing.
          if (menuItems(target).length === 0) return;
          setMenu({ target, x, y });
        }}
      />
      <div
        onMouseDown={onSidebarHandleDown}
        aria-label="Resize library tree"
        className="w-1.5 shrink-0 cursor-col-resize bg-transparent transition-colors hover:bg-primary/40"
      />
      <EditorArea
        tabs={tabs}
        activeTabIndex={activeTabIndex}
        onSelectTab={setActiveTabIndex}
        onCloseTab={closeTab}
        activeTab={activeTab}
        entries={entries}
        folders={folders}
        projects={projects}
        onRefetchIndex={refetch}
        onRefetchFolders={refetchFolders}
        onProjectChanged={onProjectChanged}
      />
      <RegisterProjectModal
        open={registerOpen}
        onClose={() => setRegisterOpen(false)}
        onRegistered={onProjectChanged}
      />
      <MaintainCLModal
        open={maintainOpen}
        onClose={() => setMaintainOpen(false)}
      />
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          items={menuItems(menu.target)}
          onClose={() => setMenu(null)}
        />
      )}
      {action && (
        <ActionModal
          {...actionModalConfig(action)}
          busy={actionBusy}
          error={actionError}
          onConfirm={runAction}
          onClose={() => {
            setAction(null);
            setActionError(null);
          }}
        />
      )}
    </div>
  );
}
