import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTauriQuery } from "../hooks/useInvoke";
import type {
  ClFolderView,
  ClIndexEntryView,
  ClRescanReportView,
  ProjectView,
} from "../lib/bindings";
import { collapseKey, tabKey, type OpenTab } from "./contextLibraryShared";
import { WorkspaceSidebar } from "./ContextLibrarySidebar";
import { EditorArea } from "./ContextLibraryEditor";
import { RegisterProjectModal } from "./ContextLibraryRegisterModal";

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
// File-content saves are disabled in v1 — `cl_write_file` doesn't exist.
// Description saves work via the existing `cl_set_description` command.
// ============================================================================

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
  const [registerOpen, setRegisterOpen] = useState(false);

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
    <div className="flex h-full bg-background">
      <WorkspaceSidebar
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
        onRescan={handleRescan}
        collapsed={collapsed}
        onToggle={toggle}
        activeTab={activeTab}
        onOpenFile={openFile}
        onOpenFolder={openFolder}
        onRequestRegister={() => setRegisterOpen(true)}
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
    </div>
  );
}
