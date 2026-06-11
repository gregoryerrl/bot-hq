import { useEffect, useMemo, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { useTauriQuery, useTauriMutation } from "../hooks/useInvoke";
import { SessionTile } from "../components/SessionTile";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import type {
  AgentConfigView,
  ClaudeConfigView,
  ClaudeOverrides,
  ModelView,
  ProjectView,
  SessionInfo,
  SessionTrayView,
} from "../lib/bindings";
import { cn } from "../lib/cn";
import { useFocusTrap } from "../hooks/useFocusTrap";
import { AgentEffortOverride } from "./ClaudeConfig";

const RAIN_DISABLED_DEFAULT_KEY = "rain_disabled_default";

/**
 * Thin wrapper that drives the per-session phase query, so `SessionTile`
 * stays pure presentational (test-friendly without a QueryClient). Each
 * loader is its own hook call — fine for the typical bot-hq session count
 * (< 20). React Query dedupes by `["get_session_phase", { sessionId }]`.
 */
function SessionTileLoader({
  session,
  pendingCount,
}: {
  session: SessionInfo;
  pendingCount: number;
}) {
  const { data: phase = null } = useTauriQuery<string | null>(
    "get_session_phase",
    { sessionId: session.id },
  );
  return (
    <SessionTile session={session} pendingCount={pendingCount} phase={phase} />
  );
}

export function Dashboard() {
  const {
    data: sessions = [],
    refetch,
    isLoading,
    error,
  } = useTauriQuery<SessionInfo[]>("list_sessions");

  // Durable pending-tray rows for all open sessions — the same source the
  // header bell uses. Survives restart and includes halt waits
  // (mark_awaiting_user / phase-advance), unlike the in-memory pending map.
  const { data: pending = [] } = useTauriQuery<SessionTrayView[]>(
    "list_pending_tray",
    {},
  );

  // Project dropdown source for the New Session dialog. Refreshes on a
  // 60s interval so newly-imported projects show without a manual reload.
  const { data: projects = [] } = useTauriQuery<ProjectView[]>(
    "list_projects",
    {},
    { refetchInterval: 60_000 },
  );

  // Saved models for the per-agent pickers, plus the configured defaults.
  const { data: models = [] } = useTauriQuery<ModelView[]>(
    "list_models",
    {},
    { refetchInterval: 60_000 },
  );
  // Each agent's configured model (Agents tab) is its default for new sessions.
  const { data: brianConfig } = useTauriQuery<AgentConfigView | null>(
    "get_agent_config",
    { agentName: "brian" },
  );
  const { data: rainConfig } = useTauriQuery<AgentConfigView | null>(
    "get_agent_config",
    { agentName: "rain" },
  );
  const { data: rainDisabledDefault } = useTauriQuery<string | null>(
    "get_app_setting",
    { key: RAIN_DISABLED_DEFAULT_KEY },
  );
  // Worktree isolation default (Settings → Agents → Session defaults).
  // Anything but "0" means on.
  const { data: worktreeDefault } = useTauriQuery<string | null>(
    "get_app_setting",
    { key: "worktree_default" },
  );

  // Persistent effort defaults, so the dialog's "Inherit" option can show what
  // it resolves to (e.g. "Inherit (max)") rather than a bare "(default)".
  // Mirrors the spawn fall-through: per-agent override > _all > settings.json
  // env. Called exactly as ClaudeConfig does (no args) so the React Query cache
  // is shared — a cache-hit if the Settings → Claude Config tab was opened.
  const { data: claudeOverrides } =
    useTauriQuery<ClaudeOverrides>("get_claude_overrides");
  const { data: claudeConfig } =
    useTauriQuery<ClaudeConfigView>("claude_config_read");
  const inheritedEffort = useMemo(() => {
    const knob =
      claudeConfig?.core_knobs.find(
        (k) => k.key === "env.CLAUDE_CODE_EFFORT_LEVEL",
      )?.value ?? null;
    const at = (a: "brian" | "rain") =>
      claudeOverrides?.[a]?.effort ?? claudeOverrides?._all?.effort ?? knob ?? null;
    return { brian: at("brian"), rain: at("rain") };
  }, [claudeOverrides, claudeConfig]);

  const createSession = useTauriMutation<
    SessionInfo,
    {
      id: string;
      title: string;
      repoPath: string | null;
      project: string | null;
      rainEnabled: boolean;
      brianModelId: string | null;
      rainModelId: string | null;
      // Effort/ultracode/worktree picks (bundled — at the tauri 10-arg limit).
      options: {
        brianEffort: string | null;
        rainEffort: string | null;
        brianUltracode: boolean | null;
        rainUltracode: boolean | null;
        useWorktree: boolean | null;
      };
    }
  >("create_session");

  const pendingBySession = useMemo(() => {
    const acc: Record<string, number> = {};
    for (const p of pending) {
      acc[p.session_id] = (acc[p.session_id] ?? 0) + 1;
    }
    return acc;
  }, [pending]);

  const [creating, setCreating] = useState(false);
  const dialogRef = useFocusTrap<HTMLDivElement>(creating);

  // ⌘/Ctrl-N lands here as `/?new=1` (see Shell) — open the dialog and eat
  // the param so refresh/back doesn't re-open it.
  const [searchParams, setSearchParams] = useSearchParams();
  useEffect(() => {
    if (searchParams.get("new") === "1") {
      setCreating(true);
      setSearchParams({}, { replace: true });
    }
  }, [searchParams, setSearchParams]);
  const [title, setTitle] = useState("");
  // Selected project name (matches ProjectView.name). Empty string = no
  // project (no working repo). When set, we look up the project's
  // working_repo_path and pass it as repoPath to create_session.
  const [selectedProject, setSelectedProject] = useState("");
  const [filter, setFilter] = useState("");
  // Per-agent model picks ("" = fall back to the agent's saved config) and the
  // Rain toggle. Seeded from the configured defaults when the dialog opens.
  const [brianModelId, setBrianModelId] = useState("");
  const [rainModelId, setRainModelId] = useState("");
  const [disableRain, setDisableRain] = useState(false);
  // Worktree isolation for this session (seeded from the app default).
  const [useWorktree, setUseWorktree] = useState(true);
  // Per-session effort/ultracode picks (null = inherit the Settings defaults).
  const [brianEffort, setBrianEffort] = useState<string | null>(null);
  const [rainEffort, setRainEffort] = useState<string | null>(null);
  const [brianUltracode, setBrianUltracode] = useState<boolean | null>(null);
  const [rainUltracode, setRainUltracode] = useState<boolean | null>(null);

  // Case-insensitive substring filter on session title. In-memory so no
  // debounce needed — the list isn't a paginated query.
  const filteredSessions = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return sessions;
    return sessions.filter((s) => s.title.toLowerCase().includes(q));
  }, [sessions, filter]);

  // Seed the model pickers + Rain toggle from the configured defaults each time
  // the dialog opens (not on every query change, so user edits aren't clobbered).
  useEffect(() => {
    if (!creating) return;
    const modelIdFor = (cfg: AgentConfigView | null | undefined) =>
      models.find(
        (m) =>
          m.provider === cfg?.provider &&
          m.model_name === cfg?.model_name &&
          (m.base_url ?? "") === (cfg?.base_url ?? ""),
      )?.id ?? "";
    setBrianModelId(modelIdFor(brianConfig));
    setRainModelId(modelIdFor(rainConfig));
    setDisableRain(rainDisabledDefault === "1");
    setUseWorktree(worktreeDefault !== "0");
    // Effort/ultracode default to inherit (the Settings defaults) each open.
    setBrianEffort(null);
    setRainEffort(null);
    setBrianUltracode(null);
    setRainUltracode(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [creating]);

  const handleCreate = async () => {
    if (!title.trim()) return;
    const id = `s-${crypto.randomUUID().slice(0, 8)}`;
    const proj = projects.find((p) => p.name === selectedProject);
    await createSession.mutateAsync({
      id,
      title: title.trim(),
      repoPath: proj?.working_repo_path ?? null,
      project: selectedProject || null,
      rainEnabled: !disableRain,
      brianModelId: brianModelId || null,
      rainModelId: disableRain ? null : rainModelId || null,
      options: {
        brianEffort,
        rainEffort: disableRain ? null : rainEffort,
        brianUltracode,
        rainUltracode: disableRain ? null : rainUltracode,
        useWorktree,
      },
    });
    setTitle("");
    setSelectedProject("");
    setCreating(false);
    refetch();
  };

  // Escape-to-dismiss + first-input focus when the dialog opens.
  const dialogTitleRef = useRef<HTMLInputElement | null>(null);
  useEffect(() => {
    if (!creating) return;
    dialogTitleRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        setCreating(false);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [creating]);

  return (
    <div className="mx-auto h-full max-w-6xl overflow-auto px-6 py-6">
      <div className="mb-6 flex items-center justify-between">
        <div>
          <h1 className="font-headline-lg text-headline-lg text-on-surface">Sessions</h1>
          <p className="mt-1 font-code-sm text-code-sm text-on-surface-variant">
            {filter.trim()
              ? `${filteredSessions.length} of ${sessions.length} match`
              : `${sessions.length} active`}
          </p>
        </div>
        <Button variant="primary" onClick={() => setCreating(true)}>
          + New session
        </Button>
      </div>
      {creating && (
        <>
          {/* Scrim — click anywhere outside the dialog to dismiss */}
          <div
            className="fixed inset-0 z-40 bg-black/60"
            onClick={() => setCreating(false)}
            aria-hidden
          />
          <div
            ref={dialogRef}
            tabIndex={-1}
            role="dialog"
            aria-modal="true"
            aria-label="New session"
            className={cn(
              "fixed left-1/2 top-1/2 z-50 w-[min(480px,90vw)] -translate-x-1/2 -translate-y-1/2",
              "rounded-lg border border-outline-variant bg-surface-container p-5 shadow-2xl focus:outline-none",
            )}
          >
            <div className="mb-4 flex items-center justify-between">
              <h2 className="font-headline-md text-headline-md text-on-surface">
                New session
              </h2>
              <button
                type="button"
                onClick={() => setCreating(false)}
                aria-label="Close"
                className="text-on-surface-variant hover:text-on-surface"
              >
                ×
              </button>
            </div>
            <div className="space-y-3">
              <label className="block">
                <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
                  Title
                </span>
                <Input
                  ref={dialogTitleRef}
                  value={title}
                  onChange={(e) => setTitle(e.target.value)}
                  placeholder="e.g., refactor auth flow"
                  onKeyDown={(e) => {
                    if (e.key === "Enter") handleCreate();
                  }}
                />
              </label>
              <label className="block">
                <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
                  Project
                </span>
                <select
                  value={selectedProject}
                  onChange={(e) => setSelectedProject(e.target.value)}
                  className={cn(
                    "w-full rounded-md border border-outline-variant bg-surface px-3 py-1.5 font-body-md text-body-md text-on-surface",
                    "focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary",
                  )}
                >
                  <option value="">(none — no working repo)</option>
                  {projects.map((p) => (
                    <option key={p.name} value={p.name}>
                      {p.display_name || p.name}
                      {p.working_repo_path ? ` — ${p.working_repo_path}` : ""}
                    </option>
                  ))}
                </select>
                <span className="mt-1 block font-code-sm text-code-sm text-on-surface-variant">
                  Drives git diff in the Apply tab + project-specific
                  policy. Leave blank for ad-hoc scopes.
                </span>
              </label>
              <label className="block">
                <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
                  Brian model
                </span>
                <select
                  value={brianModelId}
                  onChange={(e) => setBrianModelId(e.target.value)}
                  className={cn(
                    "w-full rounded-md border border-outline-variant bg-surface px-3 py-1.5 font-body-md text-body-md text-on-surface",
                    "focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary",
                  )}
                >
                  <option value="">(agent default)</option>
                  {models.map((m) => (
                    <option key={m.id} value={m.id}>
                      {m.display_name}
                    </option>
                  ))}
                </select>
              </label>
              {projects.find((p) => p.name === selectedProject)
                ?.working_repo_path && (
                <label className="flex items-center gap-2">
                  <input
                    type="checkbox"
                    checked={useWorktree}
                    onChange={(e) => setUseWorktree(e.target.checked)}
                    className="size-4 accent-primary"
                  />
                  <span className="font-body-md text-body-md text-on-surface">
                    Isolated git worktree (parallel-safe, branch{" "}
                    <code className="font-code-sm text-code-sm">bothq/…</code>)
                  </span>
                </label>
              )}
              <label className="flex items-center gap-2">
                <input
                  type="checkbox"
                  checked={disableRain}
                  onChange={(e) => setDisableRain(e.target.checked)}
                  className="size-4 accent-primary"
                />
                <span className="font-body-md text-body-md text-on-surface">
                  Disable Rain (solo Brian — saves credits)
                </span>
              </label>
              {!disableRain && (
                <label className="block">
                  <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
                    Rain model
                  </span>
                  <select
                    value={rainModelId}
                    onChange={(e) => setRainModelId(e.target.value)}
                    className={cn(
                      "w-full rounded-md border border-outline-variant bg-surface px-3 py-1.5 font-body-md text-body-md text-on-surface",
                      "focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary",
                    )}
                  >
                    <option value="">(agent default)</option>
                    {models.map((m) => (
                      <option key={m.id} value={m.id}>
                        {m.display_name}
                      </option>
                    ))}
                  </select>
                </label>
              )}
              <div>
                <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
                  Effort & ultracode (this session)
                </span>
                <div className="flex flex-col gap-2">
                  <AgentEffortOverride
                    title="Brian"
                    roleLabel="HANDS"
                    ov={{ effort: brianEffort, ultracode: brianUltracode }}
                    patch={(p) => {
                      if ("effort" in p) setBrianEffort(p.effort ?? null);
                      if ("ultracode" in p) setBrianUltracode(p.ultracode ?? null);
                    }}
                    inheritedEffort={inheritedEffort.brian}
                    isEyes={false}
                  />
                  {!disableRain && (
                    <AgentEffortOverride
                      title="Rain"
                      roleLabel="EYES"
                      ov={{ effort: rainEffort, ultracode: rainUltracode }}
                      patch={(p) => {
                        if ("effort" in p) setRainEffort(p.effort ?? null);
                        if ("ultracode" in p) setRainUltracode(p.ultracode ?? null);
                      }}
                      inheritedEffort={inheritedEffort.rain}
                      isEyes={true}
                    />
                  )}
                </div>
                <span className="mt-1 block font-code-sm text-code-sm text-on-surface-variant">
                  Overrides the Settings defaults for this session only. Leave on
                  Inherit to use your configured defaults.
                </span>
              </div>
            </div>
            <div className="mt-5 flex justify-end gap-2">
              <Button variant="ghost" onClick={() => setCreating(false)}>
                Cancel
              </Button>
              <Button
                variant="primary"
                onClick={handleCreate}
                disabled={!title.trim() || createSession.isPending}
              >
                {createSession.isPending ? "Creating…" : "Create session"}
              </Button>
            </div>
          </div>
        </>
      )}
      {sessions.length > 0 && (
        <div className="relative mb-4">
          <Input
            placeholder="Filter sessions by title…"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            className="w-full pr-8"
          />
          {filter.length > 0 && (
            <button
              type="button"
              onClick={() => setFilter("")}
              aria-label="Clear filter"
              title="Clear filter"
              className="absolute inset-y-0 right-0 flex w-8 items-center justify-center text-on-surface-variant hover:text-on-surface"
            >
              ×
            </button>
          )}
        </div>
      )}
      {error && (
        <div className="mb-6 rounded-lg border border-error/40 bg-error-container/30 px-4 py-3">
          <p className="text-sm text-on-error-container">
            Failed to load sessions: {error.message}
          </p>
          <button
            onClick={() => refetch()}
            className="mt-1 text-xs text-on-error-container underline hover:text-error"
          >
            Retry
          </button>
        </div>
      )}
      {isLoading ? (
        <div className="grid grid-cols-1 gap-gutter md:grid-cols-2 xl:grid-cols-3">
          {[0, 1, 2].map((i) => (
            <div
              key={i}
              className="h-40 animate-pulse rounded-lg border border-outline-variant bg-surface"
            />
          ))}
        </div>
      ) : sessions.length === 0 ? (
        <div className="rounded-lg border border-dashed border-outline-variant p-10 text-center">
          <p className="font-headline-md text-headline-md text-on-surface">
            Welcome to bot-hq
          </p>
          <p className="mx-auto mt-2 max-w-md text-sm text-on-surface-variant">
            A session is a scoped piece of work — Brian (HANDS) executes, Rain
            (EYES) reviews, and you stay the conductor.
          </p>
          <p className="mx-auto mt-3 max-w-md text-xs text-on-surface-variant">
            Register a project in the <b>Context Library</b> tab so sessions
            know your repo and conventions, then hit{" "}
            <b>+ New session</b> (or{" "}
            <kbd className="rounded border border-outline-variant bg-surface-container-lowest px-1 py-0.5 font-mono text-[0.65rem]">
              ⌘N
            </kbd>
            ) to put the agents to work. Repo-backed sessions run in isolated
            worktrees, so several can work the same project at once.
          </p>
        </div>
      ) : filteredSessions.length === 0 ? (
        <div className="rounded-lg border border-dashed border-outline-variant p-10 text-center">
          <p className="text-sm text-on-surface-variant">
            No sessions match <code className="font-code-sm text-code-sm">{filter.trim()}</code>.
          </p>
        </div>
      ) : (
        <div className="grid grid-cols-1 gap-gutter md:grid-cols-2 xl:grid-cols-3">
          {filteredSessions.map((s) => (
            <SessionTileLoader
              key={s.id}
              session={s}
              pendingCount={pendingBySession[s.id] ?? 0}
            />
          ))}
        </div>
      )}
    </div>
  );
}
