import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTauriQuery } from "../hooks/useInvoke";
import type {
  ClIndexEntryView,
  ClRescanReportView,
  ProjectView,
} from "../lib/bindings";
import { type OpenTab } from "./contextLibraryShared";
import { WorkspaceSidebar } from "./ContextLibrarySidebar";
import { EditorArea } from "./ContextLibraryEditor";

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
