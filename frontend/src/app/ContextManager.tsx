import { useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTauriQuery } from "../hooks/useInvoke";
import { cn } from "../lib/cn";
import type { ClRescanReportView, ProjectView } from "../lib/bindings";
import { SubTabButton } from "../components/SubTabButton";
import { ProposalQueue } from "./ProposalQueue";
import { MeasurementView } from "./MeasurementView";
import { MaintainCLModal } from "./MaintainCLModal";
import { RegisterProjectModal } from "./ContextLibraryRegisterModal";
import { RefreshIcon } from "./contextLibraryShared";
import { WrenchIcon } from "../components/icons";

// ============================================================================
// ContextManager — the management half of the Context Library: a per-project
// review surface (proposal docket + retrieval measurement), NOT a file
// explorer. Left rail lists registered projects (including `_globals`) with
// open-proposal badges; the right panel shows the selected project's docket
// and measurement under inner pills. The "Context Manager" subtab pill is the
// page header — nothing here repeats it.
// ============================================================================

/** Open-proposal counts, shared with the shell's pill badge via query key. */
export function useProposalCounts() {
  return useTauriQuery<{ project_id: string; open_count: number }[]>(
    "cl_proposal_counts",
    {},
  );
}

type ManagerTab = "proposals" | "measurement";

export function ContextManager() {
  const { data: projects = [] } = useTauriQuery<ProjectView[]>(
    "list_projects",
    {},
  );
  const { data: counts = [], refetch: refetchCounts } = useProposalCounts();
  const openByProject = useMemo(() => {
    const m: Record<string, number> = {};
    for (const c of counts) m[c.project_id] = c.open_count;
    return m;
  }, [counts]);

  // `_globals` last — it's the shared bucket, not a working project.
  const ordered = useMemo(() => {
    const named = projects.filter((p) => p.name !== "_globals");
    const globals = projects.filter((p) => p.name === "_globals");
    return [...named, ...globals];
  }, [projects]);

  const [selected, setSelected] = useState<string | null>(null);
  // Default selection: the first project with open proposals (the thing the
  // user most likely came here to handle), else the first project.
  const active =
    selected ??
    ordered.find((p) => (openByProject[p.name] ?? 0) > 0)?.name ??
    ordered[0]?.name ??
    null;
  const activeProject = ordered.find((p) => p.name === active) ?? null;

  const [tab, setTab] = useState<ManagerTab>("proposals");
  const [registerOpen, setRegisterOpen] = useState(false);
  const [maintainOpen, setMaintainOpen] = useState(false);

  const [rescanning, setRescanning] = useState(false);
  const [rescanSummary, setRescanSummary] = useState<string | null>(null);
  const handleRescan = async () => {
    if (!active || rescanning) return;
    setRescanning(true);
    setRescanSummary(null);
    try {
      const r = await invoke<ClRescanReportView>("cl_rescan", {
        project: active,
      });
      setRescanSummary(
        `+${r.added.length} ~${r.touched.length} −${r.orphaned.length}`,
      );
    } catch {
      setRescanSummary("rescan failed");
    } finally {
      setRescanning(false);
    }
  };

  return (
    <div className="flex h-full bg-background">
      <aside className="flex w-60 flex-shrink-0 flex-col border-r border-outline-variant bg-surface-container">
        <div className="min-h-0 flex-1 overflow-y-auto py-1">
          {ordered.length === 0 ? (
            <p className="px-3 py-3 font-code-sm text-code-sm text-on-surface-variant">
              No registered projects yet. Register one to manage its context.
            </p>
          ) : (
            ordered.map((p) => {
              const open = openByProject[p.name] ?? 0;
              const isActive = p.name === active;
              return (
                <button
                  key={p.name}
                  type="button"
                  onClick={() => setSelected(p.name)}
                  className={cn(
                    "flex w-full items-center justify-between gap-2 px-3 py-2 text-left font-code-sm text-code-sm transition-colors",
                    isActive
                      ? "bg-surface-container-highest text-on-surface"
                      : "text-on-surface-variant hover:bg-surface-container-high hover:text-on-surface",
                  )}
                >
                  <span className="truncate">
                    {p.display_name || p.name}
                  </span>
                  {open > 0 && (
                    <span
                      title={`${open} open proposal${open === 1 ? "" : "s"}`}
                      className="rounded-full bg-primary px-1.5 text-[10px] font-semibold leading-4 text-on-primary"
                    >
                      {open}
                    </span>
                  )}
                </button>
              );
            })
          )}
        </div>
        <div className="border-t border-outline-variant p-2">
          <button
            type="button"
            onClick={() => setRegisterOpen(true)}
            className="w-full rounded border border-outline-variant px-2 py-1.5 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:bg-surface-container-high hover:text-on-surface"
          >
            + Register project
          </button>
        </div>
      </aside>

      <div className="flex min-w-0 flex-1 flex-col">
        {activeProject == null ? (
          <div className="flex flex-1 items-center justify-center text-center">
            <p className="font-code-sm text-code-sm text-on-surface-variant">
              Select a project to review its proposals and measurement.
            </p>
          </div>
        ) : (
          <>
            <header className="flex flex-shrink-0 flex-wrap items-center justify-between gap-2 border-b border-outline-variant bg-surface-container-low px-4 py-2">
              <div className="min-w-0">
                <p className="truncate font-headline-md text-headline-md text-on-surface">
                  {activeProject.display_name || activeProject.name}
                </p>
                <p className="truncate font-code-sm text-code-sm text-on-surface-variant">
                  {activeProject.working_repo_path ?? "no working repo bound"}
                </p>
              </div>
              <div className="flex shrink-0 items-center gap-2">
                {rescanSummary && (
                  <span className="font-code-sm text-code-sm text-on-surface-variant">
                    {rescanSummary}
                  </span>
                )}
                <button
                  type="button"
                  onClick={handleRescan}
                  disabled={rescanning}
                  aria-label={`Rescan ${activeProject.name}`}
                  title={`Rescan ${activeProject.name}`}
                  className="inline-flex items-center gap-1.5 rounded border border-outline-variant px-2 py-1 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:bg-surface-container-high hover:text-on-surface disabled:opacity-50"
                >
                  <RefreshIcon
                    className={rescanning ? "animate-spin" : undefined}
                  />
                  Rescan
                </button>
                <button
                  type="button"
                  onClick={() => setMaintainOpen(true)}
                  aria-label={`Maintain CL for ${activeProject.name}`}
                  title="Dispatch an agent session to maintain this project's Context Library"
                  className="inline-flex items-center gap-1.5 rounded border border-primary/50 px-2 py-1 font-code-sm text-code-sm text-primary transition-colors hover:bg-primary/10"
                >
                  <WrenchIcon size={14} />
                  Maintain CL
                </button>
              </div>
            </header>

            <div className="flex flex-shrink-0 items-center gap-1 border-b border-outline-variant px-4">
              <SubTabButton
                active={tab === "proposals"}
                onClick={() => setTab("proposals")}
                badge={openByProject[activeProject.name] ?? 0}
              >
                Proposals
              </SubTabButton>
              <SubTabButton
                active={tab === "measurement"}
                onClick={() => setTab("measurement")}
              >
                Measurement
              </SubTabButton>
            </div>

            {tab === "proposals" ? (
              <ProposalQueue
                key={activeProject.name}
                project={activeProject.name}
                onProjectChanged={() => refetchCounts()}
              />
            ) : (
              <MeasurementView
                key={activeProject.name}
                project={activeProject.name}
              />
            )}
          </>
        )}
      </div>

      <RegisterProjectModal
        open={registerOpen}
        onClose={() => setRegisterOpen(false)}
        onRegistered={(name) => setSelected(name)}
      />
      <MaintainCLModal
        open={maintainOpen}
        onClose={() => setMaintainOpen(false)}
        initialProject={activeProject?.name}
      />
    </div>
  );
}
