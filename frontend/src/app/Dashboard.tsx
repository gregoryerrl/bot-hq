import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { useTauriQuery, useTauriMutation, errorMessage } from "../hooks/useInvoke";
import { SessionTile } from "../components/SessionTile";
import { isTrayItem } from "../components/HaltBanner";
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

/**
 * One action the USER owes — `list_user_actions` row. Declared locally
 * (matching the Rust `UserActionView` field-for-field) until the next app
 * launch regenerates bindings.ts with the new command.
 */
interface UserActionView {
  id: number;
  session_id: string;
  action: string;
  created_at: string;
  session_title: string;
  repo: string | null;
}
import { PARTICIPANT_COLORS } from "../components/authorColor";
import { useFocusTrap } from "../hooks/useFocusTrap";
import { useTauriEvent } from "../hooks/useTauriEvent";
import { pickFolder } from "./contextLibraryShared";
import {
  capabilityGapWarning,
  EDIT_FILES,
  useParticipantLabels,
} from "../lib/participants";

/**
 * How many participants this dialog offers to add.
 *
 * **It no longer mirrors the backend.** `MAX_SESSION_PARTICIPANTS`
 * (`src/tauri_cmd/sessions.rs`), which is where the limit is ENFORCED, is 8:
 * this was 2 because `spawn_session_handle` started two literally-named agents,
 * and rc3 D10 made spawn iterate the roster instead, so a third row now has a
 * process behind it.
 *
 * Left at 2 deliberately rather than raised silently — every participant is a
 * claude-code subprocess with its own context window and its own bill, so how
 * many the dialog should offer is a product call, not a consequence of this
 * change. Nothing here refuses a wider roster: a session seeded through
 * `seed_session_roster` already runs at whatever size its creator chose.
 */
export const MAX_PARTICIPANTS = 4;

/** Effort levels claude-code accepts. Mirrors `EFFORT_OPTS` in ClaudeConfig. */
const EFFORT_OPTS = ["low", "medium", "high", "xhigh", "max"];

/** One row of the dialog's participant list. */
type ParticipantRow = {
  /** Stable React key — rows are added, removed and reordered by index. */
  key: number;
  /** `null` until a role is chosen; Create stays disabled until it is not. */
  roleId: number | null;
  /** `""` = inherit the role's default model (rc3 D8). */
  modelId: string;
  /** rc3 **D12**: effort is per participant. `null` = inherit the default. */
  effort: string | null;
  /** rc3 **D12**: ultracode is per participant. `null` = inherit. */
  ultracode: boolean | null;
  /** rc3 **D20**: the palette entry by NAME, or `null` to take the rotation —
   *  which already guarantees no two participants share a hue. */
  color: string | null;
  /** rc3 **D20** (migration 0053): the name the user gave this participant.
   *  `""` means "take the ordinal", which is what the placeholder shows. Sent
   *  as `null` so the column stores an absence rather than an empty string. */
  label: string;
};

let nextParticipantKey = 1;
const emptyParticipant = (): ParticipantRow => ({
  key: nextParticipantKey++,
  roleId: null,
  modelId: "",
  effort: null,
  ultracode: null,
  color: null,
  label: "",
});

// Quickview liveness throttle: collapse bursts of agent:messages:batch into at
// most one dashboard refetch per this window (see onMessageBatch in Dashboard).
const QUICKVIEW_REFRESH_THROTTLE_MS = 2500;

/**
 * Thin wrapper that drives the per-session phase + roster queries, so
 * `SessionTile` stays pure presentational (test-friendly without a
 * QueryClient). Each loader is its own hook call — fine for the typical bot-hq
 * session count (< 20). React Query dedupes by `[command, { sessionId }]`.
 *
 * The roster is here for the Quickview byline: `last_author` is a participant
 * slug, and rc3 D10 forbids showing one. The tile needs the roster to turn it
 * into `ROLE · Model`.
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
  const { labels } = useParticipantLabels(session.id);
  return (
    <SessionTile
      session={session}
      pendingCount={pendingCount}
      phase={phase}
      authorLabels={labels}
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

  // The "waiting on you" ledger (migration 0056): actions the USER owes,
  // written by agents at close_session(user_actions=[...]) and shown until
  // checked off. s-761704e8 is why: "Merge all 5 myself now" survived only
  // in prose nobody re-surfaced, and the five PRs sat unmerged.
  // `?? []`, not a destructure default: a mock/backends returning null (not
  // undefined) skips the default and `.length` on null crashed two unrelated
  // tests — the jsdom harness lesson, relearned locally.
  const { data: userActionsRaw, refetch: refetchUserActions } =
    useTauriQuery<UserActionView[] | null>("list_user_actions", {});
  const userActions = userActionsRaw ?? [];
  const completeAction = useTauriMutation<void, { id: number }>(
    "complete_user_action",
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
  // the backend; everything else is invitable.
  //
  // **`on_mention` roles are offered as of rc3 D17.** They used to be filtered
  // out here because nothing could wake one; the user can now summon one by
  // name, so hiding them would hide the whole mode.
  const { data: roles = [] } = useTauriQuery<RoleView[]>("list_roles", {
    includeArchived: false,
  });
  const invitableRoles = roles;
  // Worktree isolation default (Settings → Agents → Session defaults).
  // Anything but "0" means on.
  const { data: worktreeDefault } = useTauriQuery<string | null>(
    "get_app_setting",
    { key: "worktree_default" },
  );

  // Persistent effort defaults, so a row's "Inherit" option can show what it
  // resolves to (e.g. "Inherit (max)") rather than a bare "(default)".
  // Mirrors the spawn fall-through: per-slot override > _all > settings.json
  // env. Called exactly as ClaudeConfig does (no args) so the React Query cache
  // is shared — a cache-hit if the Settings → Claude Config tab was opened.
  const { data: claudeOverrides } =
    useTauriQuery<ClaudeOverrides>("get_claude_overrides");
  const { data: claudeConfig } =
    useTauriQuery<ClaudeConfigView>("claude_config_read");
  /**
   * What a row's effort inherits if left alone, given the ROLE it picked.
   *
   * `claude-overrides.json` keys its per-agent scopes by role slug — the same
   * string `resolve_participant_overrides` (`src/core/session.rs`) walks
   * participant → role to produce — so this reads the entry spawn will actually
   * resolve. A row with no role yet, and a role with no entry, both fall
   * through to `_all`, matching `resolve_agent_overrides`.
   */
  const inheritedEffortFor = useMemo(() => {
    const knob =
      claudeConfig?.core_knobs.find(
        (k) => k.key === "env.CLAUDE_CODE_EFFORT_LEVEL",
      )?.value ?? null;
    return (roleId: number | null) => {
      const slug =
        roleId === null ? null : roles.find((r) => r.id === roleId)?.slug ?? null;
      const perRole = slug ? claudeOverrides?.per_role?.[slug] : undefined;
      return perRole?.effort ?? claudeOverrides?._all?.effort ?? knob ?? null;
    };
  }, [claudeOverrides, claudeConfig, roles]);

  const createSession = useTauriMutation<
    SessionInfo,
    {
      id: string;
      title: string;
      repoPath: string | null;
      project: string | null;
      // Null: derived from `options.participants` by the backend, which is the
      // single source now that the dialog picks participants rather than
      // toggling a second agent and choosing two models by name.
      rainEnabled: boolean | null;
      brianModelId: string | null;
      rainModelId: string | null;
      // Effort/ultracode/worktree/participant picks (bundled — at the tauri
      // 10-arg limit).
      options: {
        // Slot projections of `participants[0]` / `participants[1]`, kept
        // because they are the columns spawn reads TODAY. Not a second source
        // — nothing writes them but the projection in `handleCreate`, so they
        // cannot disagree with the roster they are derived from.
        brianEffort: string | null;
        rainEffort: string | null;
        brianUltracode: boolean | null;
        rainUltracode: boolean | null;
        useWorktree: boolean | null;
        participants: {
          roleId: number;
          modelId: string | null;
          // rc3 D12 — per-participant, per the shared contract.
          effort: string | null;
          ultracode: boolean | null;
          color: string | null;
          label: string | null;
        }[];
      };
    }
  >("create_session");

  const pendingBySession = useMemo(() => {
    // rc3 D35: tiles count tray QUESTIONS only — a halt is the session's
    // declared state, an approval is the gate.
    const acc: Record<string, number> = {};
    for (const p of pending) {
      if (!isTrayItem(p)) continue;
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
    // Each row's effort/ultracode reset with it — `emptyParticipant()` opens
    // them on Inherit, so there is nothing else to clear here.
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

  /** The role a row picked, or null while it has none. */
  const roleAt = (row: ParticipantRow) =>
    roles.find((r) => r.id === row.roleId) ?? null;

  /**
   * What this row would be called with no label — the placeholder for the Name
   * field.
   *
   * Mirrors the backend's ordinal rule (`participant_display_name` +
   * `slug_ordinal`) over the rows ABOVE this one: the first of a role is bare,
   * the second is `-2`. It is a hint, not the stored value — the name that
   * ships is whatever `participant_display_name` derives from the slug the
   * backend assigns, so a placeholder that guessed wrong costs a hint and never
   * a name.
   */
  const ordinalNameFor = (index: number): string => {
    const role = roleAt(participants[index]);
    if (!role) return "Name";
    const nth = participants
      .slice(0, index)
      .filter((r) => r.roleId === role.id).length;
    return nth === 0 ? role.display_name : `${role.display_name}-${nth + 1}`;
  };

  /**
   * rc3 **D11** — what the picked roster, taken together, cannot do.
   *
   * Computed from ticked capability boxes only: rows with no role yet are
   * excluded, so a half-filled dialog makes no claim. Advisory — Create is
   * never disabled by it (see the button's `disabled`, which this does not
   * appear in).
   */
  const rosterWarning = useMemo(() => {
    const picked = participants
      .map((row) => roles.find((r) => r.id === row.roleId))
      .filter((r): r is RoleView => r !== undefined);
    if (picked.length !== participants.length) return null;
    return capabilityGapWarning(picked);
  }, [participants, roles]);

  const handleCreate = async () => {
    if (!title.trim() || !rosterReady) return;
    const id = `s-${crypto.randomUUID().slice(0, 8)}`;
    const proj = projects.find((p) => p.name === selectedProject);
    // Ad-hoc repo wins over the dropdown; project stays null so the backend
    // derives it from the path basename (general policy tier).
    const repoPath = adHocRepo.trim() || proj?.working_repo_path || null;
    const project = adHocRepo.trim() ? null : selectedProject || null;
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
          // rc3 D12: the ROW is the source. The per-slot fields below are a
          // mechanical projection of rows 0 and 1 — the columns spawn reads
          // today are still per-slot, so dropping them here would silently
          // stop honouring effort until the backend reads the new fields.
          brianEffort: participants[0]?.effort ?? null,
          rainEffort: participants[1]?.effort ?? null,
          brianUltracode: participants[0]?.ultracode ?? null,
          rainUltracode: participants[1]?.ultracode ?? null,
          useWorktree,
          participants: participants.map((p) => ({
            roleId: p.roleId as number,
            modelId: p.modelId || null,
            effort: p.effort,
            ultracode: p.ultracode,
            color: p.color,
            // Blank is an absence, not a name — the backend trims and falls
            // back to the ordinal either way, but storing `""` would make an
            // untouched input indistinguishable from a cleared one.
            label: p.label.trim() || null,
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
                        {/* rc3 D12: effort belongs to the PARTICIPANT, next to
                            the role and model it applies to — not to a fixed
                            block named after an agent. */}
                        <label className="block">
                          <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
                            Effort
                          </span>
                          <select
                            aria-label={`Participant ${index + 1} effort`}
                            value={row.effort ?? ""}
                            onChange={(e) =>
                              patchParticipant(index, {
                                effort: e.target.value || null,
                                // `max` and ultracode are mutually exclusive in
                                // claude-code, so picking `max` clears it.
                                ...(e.target.value === "max"
                                  ? { ultracode: null }
                                  : {}),
                              })
                            }
                            className={cn(
                              "w-full rounded-md border border-outline-variant bg-surface px-3 py-1.5 font-body-md text-body-md text-on-surface",
                              "focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary",
                            )}
                          >
                            <option value="">
                              Inherit
                              {inheritedEffortFor(row.roleId)
                                ? ` (${inheritedEffortFor(row.roleId)})`
                                : " (default)"}
                            </option>
                            {EFFORT_OPTS.map((v) => (
                              <option key={v} value={v}>
                                {v}
                              </option>
                            ))}
                          </select>
                        </label>
                        {/* Ultracode rides in on `--settings`, which spawn
                            injects only for a role holding `edit_files` — so
                            offering it to a role without that box would be a
                            control that changes nothing. Capability, not role
                            name: bot-hq does not know what a role means. */}
                        <label className="flex items-end gap-2 pb-1.5">
                          <input
                            type="checkbox"
                            aria-label={`Participant ${index + 1} ultracode`}
                            checked={row.ultracode === true}
                            disabled={
                              !roleAt(row)?.capabilities.includes(EDIT_FILES) ||
                              row.effort === "max"
                            }
                            onChange={(e) =>
                              patchParticipant(index, {
                                ultracode: e.target.checked ? true : null,
                              })
                            }
                            className="size-4 accent-primary"
                          />
                          <span className="font-code-sm text-code-sm text-on-surface">
                            Ultracode
                          </span>
                        </label>
                        {/* rc3 D20's other half (migration 0053): the NAME
                            this participant goes by. Empty takes the ordinal,
                            which is what the placeholder shows — so the field
                            reads as an override of something that already
                            works, not as a blank that has to be filled. */}
                        <label className="block">
                          <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
                            Name
                          </span>
                          <input
                            type="text"
                            value={row.label}
                            placeholder={ordinalNameFor(index)}
                            aria-label={`Participant ${index + 1} name`}
                            onChange={(e) =>
                              patchParticipant(index, { label: e.target.value })
                            }
                            className="w-full rounded border border-outline-variant bg-surface px-2 py-1 font-code-sm text-code-sm text-on-surface placeholder:text-on-surface-variant"
                          />
                        </label>
                        {/* rc3 D20: the colour this participant's byline takes.
                            "Rotate" is the DEFAULT rather than an absence — the
                            rotation already guarantees no two participants of
                            one session share a hue, so a pick is a preference,
                            not a fix. Swatches rather than a select: the thing
                            being chosen is a colour, and its name is the label
                            of last resort. */}
                        <label className="block">
                          <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
                            Colour
                          </span>
                          <div className="flex flex-wrap items-center gap-1 pt-0.5">
                            <button
                              type="button"
                              aria-label={`Participant ${index + 1} colour: rotate`}
                              aria-pressed={row.color === null}
                              title="Rotate — a distinct hue by turn order"
                              onClick={() => patchParticipant(index, { color: null })}
                              className={cn(
                                "size-5 rounded-full border text-[0.6rem] leading-none text-on-surface-variant",
                                row.color === null
                                  ? "border-primary ring-1 ring-primary"
                                  : "border-outline-variant",
                              )}
                            >
                              ↻
                            </button>
                            {PARTICIPANT_COLORS.map((c) => (
                              <button
                                key={c.name}
                                type="button"
                                aria-label={`Participant ${index + 1} colour: ${c.name}`}
                                aria-pressed={row.color === c.name}
                                title={c.name}
                                onClick={() => patchParticipant(index, { color: c.name })}
                                className={cn(
                                  "size-5 rounded-full border",
                                  c.swatch,
                                  row.color === c.name
                                    ? "border-primary ring-1 ring-primary"
                                    : "border-outline-variant",
                                )}
                              />
                            ))}
                          </div>
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
                  Row order sets each participant's turn slot. Effort applies to
                  this session only — leave it on Inherit to use your configured
                  defaults.
                </p>
                {/* rc3 D11. Advisory, never blocking: it names a gap in what
                    the roster's ticked boxes allow, and the user decides
                    whether that is what they meant. */}
                {rosterWarning && (
                  <p
                    role="status"
                    className="mt-2 rounded border border-warning/50 bg-warning/15 px-3 py-2 font-code-sm text-code-sm text-warning"
                  >
                    ⚠ {rosterWarning}
                  </p>
                )}
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
      {userActions.length > 0 && (
        <div
          data-testid="waiting-on-you"
          className="mb-6 rounded-lg border border-outline-variant bg-surface px-4 py-3"
        >
          <p className="text-sm font-medium text-on-surface">Waiting on you</p>
          <ul className="mt-2 space-y-1.5">
            {userActions.map((a) => (
              <li
                key={a.id}
                className="flex items-start gap-2 text-sm text-on-surface-variant"
              >
                <input
                  type="checkbox"
                  aria-label={`Done: ${a.action}`}
                  className="mt-0.5 accent-primary"
                  disabled={completeAction.isPending}
                  onChange={() =>
                    completeAction.mutate(
                      { id: a.id },
                      { onSuccess: () => refetchUserActions() },
                    )
                  }
                />
                <span>
                  {a.action}
                  <span className="ml-2 text-xs opacity-70">
                    {a.repo ? a.repo.split("/").pop() : a.session_title}
                  </span>
                </span>
              </li>
            ))}
          </ul>
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
            A session is a scoped piece of work — you pick which of your roles
            take part, what each one runs on, and you stay the conductor.
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
                    ) and choose its participants.
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
