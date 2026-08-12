import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { useTauriQuery, useTauriMutation, errorMessage } from "../hooks/useInvoke";
import { SessionTile } from "../components/SessionTile";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import type {
  ClaudeConfigView,
  ClaudeOverrides,
  ModelView,
  ProjectView,
  RoleView,
  SessionInfo,
  SessionTrayView,
} from "../lib/bindings";
import { cn } from "../lib/cn";
import { useFocusTrap } from "../hooks/useFocusTrap";
import { useTauriEvent } from "../hooks/useTauriEvent";
import { AgentEffortOverride } from "./ClaudeConfig";
import { pickFolder } from "./contextLibraryShared";

/**
 * How many participants a session can be created with.
 *
 * Mirrors `MAX_SESSION_PARTICIPANTS` in `src/tauri_cmd/sessions.rs`, which is
 * where it is ENFORCED — this constant only stops the dialog offering a row the
 * backend would refuse. It is the runtime's limit, not a design one: spawn
 * still starts two literally-named agents, so a third participant would have no
 * process behind it.
 */
const MAX_PARTICIPANTS = 2;

/** One row of the dialog's participant list. */
type ParticipantRow = {
  /** Stable React key — rows are added, removed and reordered by index. */
  key: number;
  /** `null` until a role is chosen; Create stays disabled until it is not. */
  roleId: number | null;
  /** `""` = inherit the role's default model (rc3 D8). */
  modelId: string;
};

let nextParticipantKey = 1;
const emptyParticipant = (): ParticipantRow => ({
  key: nextParticipantKey++,
  roleId: null,
  modelId: "",
});

// Quickview liveness throttle: collapse bursts of agent:messages:batch into at
// most one dashboard refetch per this window (see onMessageBatch in Dashboard).
const QUICKVIEW_REFRESH_THROTTLE_MS = 2500;

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

  // Quickview liveness: agent:messages:batch fires on every message batch.
  // Throttle a list_sessions refetch so the dashboard's per-tile Quickview
  // stays current while watched, without re-running the per-session preview
  // query on every batch. Local to the dashboard — the listener (and its cost)
  // unmounts with it; that's why Providers.tsx leaves this event out of the
  // global invalidation map.
  const lastQuickviewRefreshRef = useRef(0);
  const quickviewRefreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  const onMessageBatch = useCallback(() => {
    const now = Date.now();
    const sinceLast = now - lastQuickviewRefreshRef.current;
    if (sinceLast >= QUICKVIEW_REFRESH_THROTTLE_MS) {
      lastQuickviewRefreshRef.current = now;
      refetch();
    } else if (quickviewRefreshTimerRef.current == null) {
      // Trailing edge: reflect the tail of a burst once the window elapses.
      quickviewRefreshTimerRef.current = setTimeout(() => {
        quickviewRefreshTimerRef.current = null;
        lastQuickviewRefreshRef.current = Date.now();
        refetch();
      }, QUICKVIEW_REFRESH_THROTTLE_MS - sinceLast);
    }
  }, [refetch]);
  useTauriEvent("agent:messages:batch", onMessageBatch, [onMessageBatch]);
  useEffect(
    () => () => {
      if (quickviewRefreshTimerRef.current) {
        clearTimeout(quickviewRefreshTimerRef.current);
      }
    },
    [],
  );

  // Durable pending-tray rows for all open sessions — the same source the
  // header bell uses. Survives restart and includes halt waits
  // (mark_awaiting_user / phase-advance), unlike the in-memory pending map.
  const { data: pending = [] } = useTauriQuery<SessionTrayView[]>(
    "list_pending_tray",
    {},
  );

  // Project dropdown source for the New Session dialog. Refreshed live via the
  // `project:changed` event (project register/unregister) — no poll needed.
  const { data: projects = [] } = useTauriQuery<ProjectView[]>(
    "list_projects",
    {},
  );

  // Saved models for the per-agent pickers. Refreshed live via the
  // `model:changed` event (upsert/delete) — no poll needed.
  const { data: models = [] } = useTauriQuery<ModelView[]>(
    "list_models",
    {},
  );
  // The roles a participant can be invited from. Archived ones are excluded by
  // the backend, and `on_demand` ones are filtered below: waking one needs the
  // user `@mention` that rc3 D1 defers, so inviting one would produce a
  // participant the ring skips and nothing ever wakes.
  const { data: roles = [] } = useTauriQuery<RoleView[]>("list_roles", {
    includeArchived: false,
  });
  const invitableRoles = useMemo(
    () => roles.filter((r) => r.participation_mode !== "on_demand"),
    [roles],
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
      // Null: derived from `options.participants` by the backend, which is the
      // single source now that the dialog picks participants rather than
      // toggling Rain and choosing two models by name.
      rainEnabled: boolean | null;
      brianModelId: string | null;
      rainModelId: string | null;
      // Effort/ultracode/worktree/participant picks (bundled — at the tauri
      // 10-arg limit).
      options: {
        brianEffort: string | null;
        rainEffort: string | null;
        brianUltracode: boolean | null;
        rainUltracode: boolean | null;
        useWorktree: boolean | null;
        participants: { roleId: number; modelId: string | null }[];
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
  // Inline create-session error so a rejected mutation doesn't leave the dialog
  // silently stuck on "Creating…" (the dialog stays open on failure to show it).
  const [createError, setCreateError] = useState<string | null>(null);
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
  // Ad-hoc working repo picked directly (folder not registered as a project).
  // When set it overrides the dropdown: repoPath = this path, project = null
  // (the backend derives the project from the path basename and the session
  // inherits the general policy tier, since the repo isn't a registered project).
  const [adHocRepo, setAdHocRepo] = useState("");
  const [filter, setFilter] = useState("");
  // The session's participants, in turn order. Default 1 (design §1) — the
  // list IS the running order, so row 0 takes the first turn.
  const [participants, setParticipants] = useState<ParticipantRow[]>([
    emptyParticipant(),
  ]);
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

  // Reset the dialog's picks each time it opens (not on every query change, so
  // user edits aren't clobbered).
  useEffect(() => {
    if (!creating) return;
    setCreateError(null);
    setParticipants([emptyParticipant()]);
    setUseWorktree(worktreeDefault !== "0");
    setSelectedProject("");
    setAdHocRepo("");
    // Effort/ultracode default to inherit (the Settings defaults) each open.
    setBrianEffort(null);
    setRainEffort(null);
    setBrianUltracode(null);
    setRainUltracode(null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [creating]);

  // Every row has a role. Until then Create is disabled — a participant with no
  // role is a participant with no capabilities, and guessing one for the user
  // is how a session silently gets an agent they did not choose.
  const rosterReady =
    participants.length > 0 && participants.every((p) => p.roleId !== null);

  const patchParticipant = (index: number, patch: Partial<ParticipantRow>) =>
    setParticipants((rows) =>
      rows.map((row, i) => (i === index ? { ...row, ...patch } : row)),
    );

  // The role a slot is playing, for the effort block's label. Empty until the
  // row has a role — the label is context, so it stays blank rather than
  // guessing.
  const roleLabelAt = (index: number) =>
    roles.find((r) => r.id === participants[index]?.roleId)?.display_name ?? "";

  const handleCreate = async () => {
    if (!title.trim() || !rosterReady) return;
    const id = `s-${crypto.randomUUID().slice(0, 8)}`;
    const proj = projects.find((p) => p.name === selectedProject);
    // Ad-hoc repo wins over the dropdown; project stays null so the backend
    // derives it from the path basename (general policy tier).
    const repoPath = adHocRepo.trim() || proj?.working_repo_path || null;
    const project = adHocRepo.trim() ? null : selectedProject || null;
    const solo = participants.length < 2;
    setCreateError(null);
    let ok = false;
    try {
      await createSession.mutateAsync({
        id,
        title: title.trim(),
        repoPath,
        project,
        // The roster is the source: the backend derives the solo/duo flag and
        // both model columns from `participants`, so sending them here too
        // would be a second source that can disagree with it.
        rainEnabled: null,
        brianModelId: null,
        rainModelId: null,
        options: {
          brianEffort,
          rainEffort: solo ? null : rainEffort,
          brianUltracode,
          rainUltracode: solo ? null : rainUltracode,
          useWorktree,
          participants: participants.map((p) => ({
            roleId: p.roleId as number,
            modelId: p.modelId || null,
          })),
        },
      });
      ok = true;
    } catch (e) {
      // Keep the dialog open so the inline error is visible.
      setCreateError(errorMessage(e));
    } finally {
      // Only tear the dialog down on success; on failure it stays up to show
      // the error (this guarantees we never get wedged on "Creating…").
      if (ok) {
        setTitle("");
        setSelectedProject("");
        setCreating(false);
        refetch();
      }
    }
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
    <div className="mx-auto h-full max-w-6xl overflow-y-auto overflow-x-hidden px-6 py-6">
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
              "fixed left-1/2 top-1/2 z-50 w-[min(480px,90vw)] max-h-[90vh] -translate-x-1/2 -translate-y-1/2 overflow-y-auto",
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
            <div className="space-y-4">
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
                  onChange={(e) => {
                    setSelectedProject(e.target.value);
                    if (e.target.value) setAdHocRepo("");
                  }}
                  className={cn(
                    "w-full rounded-md border border-outline-variant bg-surface px-3 py-1.5 font-body-md text-body-md text-on-surface",
                    "focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary",
                  )}
                >
                  <option value="">(none — no working repo)</option>
                  {projects.map((p) => (
                    <option key={p.name} value={p.name}>
                      {p.display_name || p.name}
                    </option>
                  ))}
                </select>
                <span className="mt-1 block font-code-sm text-code-sm text-on-surface-variant">
                  Drives git diff in the Apply tab + project-specific
                  policy. Leave blank for ad-hoc scopes.
                </span>
              </label>
              {/* Ad-hoc repo: pick a folder that isn't a registered project. */}
              {adHocRepo ? (
                <div className="flex items-center justify-between gap-2 rounded-md border border-outline-variant bg-surface px-3 py-1.5">
                  <span
                    className="truncate font-code-sm text-code-sm text-on-surface"
                    title={adHocRepo}
                  >
                    {adHocRepo}
                  </span>
                  <button
                    type="button"
                    onClick={() => setAdHocRepo("")}
                    aria-label="Clear picked folder"
                    className="shrink-0 text-on-surface-variant transition-colors hover:text-on-surface"
                  >
                    ×
                  </button>
                </div>
              ) : (
                <button
                  type="button"
                  onClick={async () => {
                    try {
                      const picked = await pickFolder("Choose a working repo");
                      if (picked) {
                        setAdHocRepo(picked);
                        setSelectedProject("");
                      }
                    } catch (e) {
                      setCreateError(errorMessage(e));
                    }
                  }}
                  className="font-code-sm text-code-sm text-primary transition-colors hover:underline"
                >
                  or pick a folder not listed…
                </button>
              )}
              {adHocRepo && (
                <p className="font-code-sm text-code-sm text-on-surface-variant">
                  Ad-hoc repo — not a registered project, so this session uses
                  the general policy tier.
                </p>
              )}
              <div>
                <div className="mb-1 flex items-center justify-between">
                  <span className="font-label-caps text-label-caps text-on-surface-variant">
                    Participants
                  </span>
                  <span className="font-code-sm text-code-sm text-on-surface-variant">
                    {participants.length} of {MAX_PARTICIPANTS}
                  </span>
                </div>
                <div className="flex flex-col gap-2">
                  {participants.map((row, index) => (
                    <div
                      key={row.key}
                      className="rounded-md border border-outline-variant bg-surface p-2"
                    >
                      <div className="mb-1 flex items-center justify-between">
                        <span className="font-code-sm text-code-sm text-on-surface-variant">
                          Participant {index + 1}
                        </span>
                        {participants.length > 1 && (
                          <button
                            type="button"
                            aria-label={`Remove participant ${index + 1}`}
                            onClick={() =>
                              setParticipants((rows) =>
                                rows.filter((_, i) => i !== index),
                              )
                            }
                            className="text-on-surface-variant transition-colors hover:text-on-surface"
                          >
                            ×
                          </button>
                        )}
                      </div>
                      <div className="grid grid-cols-2 gap-2">
                        <label className="block">
                          <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
                            Role
                          </span>
                          <select
                            aria-label={`Participant ${index + 1} role`}
                            value={row.roleId ?? ""}
                            onChange={(e) =>
                              patchParticipant(index, {
                                roleId: e.target.value
                                  ? Number(e.target.value)
                                  : null,
                              })
                            }
                            className={cn(
                              "w-full rounded-md border border-outline-variant bg-surface px-3 py-1.5 font-body-md text-body-md text-on-surface",
                              "focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary",
                            )}
                          >
                            <option value="">(choose a role)</option>
                            {invitableRoles.map((r) => (
                              <option key={r.id} value={r.id}>
                                {r.display_name}
                              </option>
                            ))}
                          </select>
                        </label>
                        <label className="block">
                          <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
                            Model
                          </span>
                          <select
                            aria-label={`Participant ${index + 1} model`}
                            value={row.modelId}
                            onChange={(e) =>
                              patchParticipant(index, { modelId: e.target.value })
                            }
                            className={cn(
                              "w-full rounded-md border border-outline-variant bg-surface px-3 py-1.5 font-body-md text-body-md text-on-surface",
                              "focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary",
                            )}
                          >
                            <option value="">(role default)</option>
                            {models.map((m) => (
                              <option key={m.id} value={m.id}>
                                {m.display_name}
                              </option>
                            ))}
                          </select>
                        </label>
                      </div>
                    </div>
                  ))}
                </div>
                {participants.length < MAX_PARTICIPANTS && (
                  <button
                    type="button"
                    onClick={() =>
                      setParticipants((rows) => [...rows, emptyParticipant()])
                    }
                    className="mt-2 font-code-sm text-code-sm text-primary transition-colors hover:underline"
                  >
                    + Add participant
                  </button>
                )}
                <p className="mt-1 font-code-sm text-code-sm text-on-surface-variant">
                  Row order sets each participant's turn slot.
                </p>
                {invitableRoles.length === 0 && (
                  <p className="mt-1 font-code-sm text-code-sm text-on-surface-variant">
                    No roles yet — add one in <b>Settings → Roles</b> before
                    starting a session.
                  </p>
                )}
                {models.length === 0 && invitableRoles.length > 0 && (
                  <p className="mt-1 font-code-sm text-code-sm text-on-surface-variant">
                    No saved models yet — each participant uses its role's
                    configured default. Add models in <b>Settings → Models</b> to
                    pick per session (and to run a pre-flight connection test).
                  </p>
                )}
                {/* rc3 D9: one connector, so the picker no longer distinguishes
                    runtimes — which means the list can offer a model whose
                    gateway the CLI cannot talk to. Say so here, where the choice
                    is made, rather than letting it surface as a spawn error. */}
                {models.length > 0 && (
                  <p className="mt-1 font-code-sm text-code-sm text-on-surface-variant">
                    Every model spawns through the claude CLI, so its endpoint
                    must speak the Anthropic Messages API. Use <b>Test</b> in{" "}
                    <b>Settings → Models</b> to check one before a session
                    depends on it.
                  </p>
                )}
              </div>
              {(projects.find((p) => p.name === selectedProject)
                ?.working_repo_path ||
                adHocRepo.trim()) && (
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
              <div>
                <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
                  Effort & ultracode (this session)
                </span>
                {/* Still per-SLOT, because the columns spawn reads are still
                    per-slot: slot 0's knobs land in `sessions.brian_effort`,
                    slot 1's in `rain_effort`. The role label follows the pick. */}
                <div className="flex flex-col gap-2">
                  <AgentEffortOverride
                    title="Brian"
                    roleLabel={roleLabelAt(0)}
                    ov={{ effort: brianEffort, ultracode: brianUltracode }}
                    patch={(p) => {
                      if ("effort" in p) setBrianEffort(p.effort ?? null);
                      if ("ultracode" in p) setBrianUltracode(p.ultracode ?? null);
                    }}
                    inheritedEffort={inheritedEffort.brian}
                    isEyes={false}
                  />
                  {participants.length > 1 && (
                    <AgentEffortOverride
                      title="Rain"
                      roleLabel={roleLabelAt(1)}
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
            {createError && (
              <p className="mt-4 rounded border border-error/40 bg-error-container/20 px-3 py-2 font-code-sm text-code-sm text-on-error-container">
                Create failed: {createError}
              </p>
            )}
            <div className="mt-5 flex justify-end gap-2">
              <Button variant="ghost" onClick={() => setCreating(false)}>
                Cancel
              </Button>
              <Button
                variant="primary"
                onClick={handleCreate}
                disabled={!title.trim() || !rosterReady || createSession.isPending}
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
          <ol className="mx-auto mt-5 max-w-sm space-y-2.5 text-left">
            {[
              {
                done: projects.length > 0,
                body: (
                  <>
                    Add a project in the <b>Context Library</b> tab (or pick a
                    repo folder when you start a session) — so sessions know your
                    repo and conventions.
                  </>
                ),
              },
              {
                done: models.length > 0,
                body: (
                  <>
                    Add a model in <b>Settings → Models</b> (optional — agents
                    use their built-in default otherwise).
                  </>
                ),
              },
              {
                done: false,
                body: (
                  <>
                    Create a session with <b>+ New session</b> (or{" "}
                    <kbd className="rounded border border-outline-variant bg-surface-container-lowest px-1 py-0.5 font-mono text-[0.65rem]">
                      ⌘N
                    </kbd>
                    ) to put Brian + Rain to work.
                  </>
                ),
              },
            ].map((step, i) => (
              <li key={i} className="flex items-start gap-2.5">
                <span
                  className={cn(
                    "mt-0.5 flex size-5 shrink-0 items-center justify-center rounded-full text-[0.7rem]",
                    step.done
                      ? "bg-success/20 text-success"
                      : "border border-outline-variant text-on-surface-variant",
                  )}
                  aria-hidden
                >
                  {step.done ? "✓" : i + 1}
                </span>
                <span className="text-xs text-on-surface-variant">
                  {step.body}
                </span>
              </li>
            ))}
          </ol>
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
