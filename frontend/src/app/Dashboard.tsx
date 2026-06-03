import { useEffect, useMemo, useRef, useState } from "react";
import { useTauriQuery, useTauriMutation } from "../hooks/useInvoke";
import { SessionTile } from "../components/SessionTile";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import type {
  ModelView,
  PendingChoiceView,
  ProjectView,
  SessionInfo,
} from "../lib/bindings";
import { cn } from "../lib/cn";

const RAIN_DISABLED_DEFAULT_KEY = "rain_disabled_default";

/**
 * Thin wrapper that drives the per-session phase query, so `SessionTile`
 * stays pure presentational (test-friendly without a QueryClient). Each
 * loader is its own hook call — fine for the typical bot-hq session count
 * (< 20). React Query dedupes by `["get_session_phase", { sessionId }]`.
 */
function SessionTileLoader({
  session,
  pendingChoices,
}: {
  session: SessionInfo;
  pendingChoices: PendingChoiceView[];
}) {
  const { data: phase = null } = useTauriQuery<string | null>(
    "get_session_phase",
    { sessionId: session.id },
    { refetchInterval: 5_000 },
  );
  return (
    <SessionTile
      session={session}
      pendingChoices={pendingChoices}
      phase={phase}
    />
  );
}

export function Dashboard() {
  const {
    data: sessions = [],
    refetch,
    isLoading,
    error,
  } = useTauriQuery<SessionInfo[]>("list_sessions");

  const { data: pending = [] } = useTauriQuery<PendingChoiceView[]>(
    "list_pending_choices",
    {},
    { refetchInterval: 5_000 },
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
  const { data: defaultModelId } = useTauriQuery<string | null>(
    "get_default_model_id",
  );
  const { data: rainDisabledDefault } = useTauriQuery<string | null>(
    "get_app_setting",
    { key: RAIN_DISABLED_DEFAULT_KEY },
  );

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
    }
  >("create_session");

  const pendingBySession = useMemo(() => {
    const acc: Record<string, PendingChoiceView[]> = {};
    for (const p of pending) {
      (acc[p.session_id] = acc[p.session_id] ?? []).push(p);
    }
    return acc;
  }, [pending]);

  const [creating, setCreating] = useState(false);
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
    setBrianModelId(defaultModelId ?? "");
    setRainModelId(defaultModelId ?? "");
    setDisableRain(rainDisabledDefault === "1");
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
            role="dialog"
            aria-modal="true"
            aria-label="New session"
            className={cn(
              "fixed left-1/2 top-1/2 z-50 w-[min(480px,90vw)] -translate-x-1/2 -translate-y-1/2",
              "rounded-lg border border-outline-variant bg-surface-container p-5 shadow-2xl",
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
          <p className="text-sm text-on-surface">
            No active sessions yet.
          </p>
          <p className="mt-1 text-xs text-on-surface-variant">
            Click <b>+ New session</b> to spawn a Brian + Rain duo on a scope.
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
              pendingChoices={pendingBySession[s.id] ?? []}
            />
          ))}
        </div>
      )}
    </div>
  );
}
