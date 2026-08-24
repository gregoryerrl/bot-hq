import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { useTauriQuery, useTauriMutation, errorMessage } from "../hooks/useInvoke";
import { SessionTile } from "../components/SessionTile";
import { isTrayItem } from "../components/HaltBanner";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import type {
  ClaudeOverrides,
  ModelView,
  ProjectView,
  RoleView,
  SessionInfo,
  SessionTrayView,
} from "../lib/bindings";
import {
  EFFORT_LEVELS,
  ULTRACODE,
  pickToFields,
  roleDefaultEffort,
} from "../lib/effort";
import { cn } from "../lib/cn";
import { PARTICIPANT_COLORS } from "../components/authorColor";
import { RescanIcon, WarnIcon } from "../components/icons";
import { useFocusTrap } from "../hooks/useFocusTrap";
import { useTauriEvent } from "../hooks/useTauriEvent";
import { pickFolder } from "./contextLibraryShared";
import { Skeleton } from "../components/ui/Skeleton";
import { selectClass } from "../components/ui/Select";
import {
  rosterAdvisory,
  EDIT_FILES,
  useParticipantLabels,
} from "../lib/participants";

/**
 * How many participants this dialog offers to add: **4** (CLAUDE.md: "dialog
 * default 1, cap 4").
 *
 * **Deliberately NOT the backend's ceiling.** `MAX_SESSION_PARTICIPANTS`
 * (`src/storage/participants.rs`, re-exported by `src/tauri_cmd/sessions.rs`),
 * where the limit is ENFORCED, is 8 — the storage/seeder cap that stops a
 * plugin or a wide roster from spawning without bound. This is the PRODUCT
 * cap: every participant is a claude-code subprocess with its own context
 * window and its own bill, so how many the dialog offers is a product call,
 * and it was raised from 2 (the two literally-named agents rc3 D10 retired) to
 * 4 on purpose. Do not "harmonise" the two numbers; a session seeded through
 * `seed_session_roster` already runs at whatever size its creator chose, up to
 * the backend's 8.
 */
export const MAX_PARTICIPANTS = 4;

/** One row of the dialog's participant list. */
type ParticipantRow = {
  /** Stable React key — rows are added, removed and reordered by index. */
  key: number;
  /** `null` until a role is chosen; Create stays disabled until it is not. */
  roleId: number | null;
  /** `""` = the role's default model (rc3 D8). */
  modelId: string;
  /** rc3 **D12**: effort is per participant. `null` = the Default choice —
   *  spawn resolves the role's configured default, else the medium floor.
   *  An ultracode pick stores `"xhigh"` here (see `lib/effort.ts`). */
  effort: string | null;
  /** rc3 **D12**: ultracode is per participant. `null` = Default; a concrete
   *  level pick stores an explicit `false` so it clears a role-default
   *  ultracode. */
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
  const { labels, hues } = useParticipantLabels(session.id);
  return (
    <SessionTile
      session={session}
      pendingCount={pendingCount}
      phase={phase}
      authorLabels={labels}
      authorHues={hues}
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
  // Worktree isolation default (Settings → Policy → Session defaults; the
  // Agents tab that used to host it was retired by rc3 D8).
  // Anything but "0" means on.
  const { data: worktreeDefault } = useTauriQuery<string | null>(
    "get_app_setting",
    { key: "worktree_default" },
  );

  // Role defaults, so a row's "Default (…)" option shows what it resolves to.
  // Called exactly as the Roles tab does (no args) so the React Query cache is
  // shared — a cache-hit if Settings → Roles was opened.
  const { data: claudeOverrides } =
    useTauriQuery<ClaudeOverrides>("get_claude_overrides");
  /**
   * What a row spawns with if left on Default, given the ROLE it picked.
   *
   * `claude-overrides.json` keys its scopes by role slug — the same string
   * `resolve_participant_overrides` (`src/core/session.rs`) walks participant →
   * role to produce — so this reads the entry spawn will actually resolve.
   * `roleDefaultEffort` mirrors the spawn floor: a row with no role yet and a
   * role with no entry both show the medium floor, never `_all` or the
   * settings.json knob (no-inherit, 2026-08-25).
   */
  const defaultEffortFor = useMemo(() => {
    return (roleId: number | null) => {
      const slug =
        roleId === null ? null : roles.find((r) => r.id === roleId)?.slug ?? null;
      return roleDefaultEffort(
        slug ? claudeOverrides?.per_role?.[slug] : undefined,
      );
    };
  }, [claudeOverrides, roles]);

  // Drag-to-SWAP (ideas.md 2026-08-24, tray c38a216b): dropping tile A on
  // tile B exchanges exactly those two slots server-side; the refetch
  // re-renders the grid in the stored order. `dragId` doubles as the "a drag
  // is live" flag; `dropTarget` only ever names a DIFFERENT tile, so the
  // ring highlight cannot appear on the tile being dragged.
  const [dragId, setDragId] = useState<string | null>(null);
  const [dropTarget, setDropTarget] = useState<string | null>(null);
  const swapOrder = useTauriMutation<boolean, { a: string; b: string }>(
    "swap_session_order",
  );
  const handleTileDrop = useCallback(
    (targetId: string) => {
      const from = dragId;
      setDragId(null);
      setDropTarget(null);
      if (!from || from === targetId) return;
      swapOrder.mutate(
        { a: from, b: targetId },
        { onSettled: () => void refetch() },
      );
    },
    [dragId, swapOrder, refetch],
  );

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
      multiParticipant: boolean | null;
      slot0ModelId: string | null;
      slot1ModelId: string | null;
      // Effort/ultracode/worktree/participant picks (bundled — at the tauri
      // 10-arg limit).
      options: {
        // Slot projections of `participants[0]` / `participants[1]`, kept
        // because they were the columns spawn read until migration 0060 (see
        // the note below). Not a second source
        // — nothing writes them but the projection in `handleCreate`, so they
        // cannot disagree with the roster they are derived from.
        slot0Effort: string | null;
        slot1Effort: string | null;
        slot0Ultracode: boolean | null;
        slot1Ultracode: boolean | null;
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
    // them on Default, so there is nothing else to clear here.
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
    return rosterAdvisory(picked);
  }, [participants, roles]);

  /**
   * The next role NOT already on the roster, for a freshly added row (round
   * 8): a second slot used to open on "(choose a role)", and the roster
   * shipped as `hands` + `hands-2` when the user picked the same role again.
   * The user can still change it — this is the default, not a rule.
   */
  const nextUnusedRoleId = (rows: readonly ParticipantRow[]): number | null => {
    const used = new Set(rows.map((r) => r.roleId));
    return invitableRoles.find((r) => !used.has(r.id))?.id ?? null;
  };

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
        // The roster is the source: the backend derives the multi-participant
        // flag and the per-slot model columns from `participants`, so sending
        // them here too would be a second source that can disagree with it.
        multiParticipant: null,
        slot0ModelId: null,
        slot1ModelId: null,
        options: {
          // rc3 D12: the ROW is the source. The per-slot fields below are a
          // mechanical projection of rows 0 and 1, kept as the fallback for a
          // caller that sends no `participants` array at all — `resolve_participant_picks`
          // applies them to slots 0 and 1 only when a pick leaves effort unset.
          //
          // They were `brianEffort` / `rainEffort` until the D10 hard
          // retirement. The justification written here then — that the columns
          // spawn reads are still per-slot — stopped being true when 0060
          // dropped those columns; spawn reads each participant's own row.
          slot0Effort: participants[0]?.effort ?? null,
          slot1Effort: participants[1]?.effort ?? null,
          slot0Ultracode: participants[0]?.ultracode ?? null,
          slot1Ultracode: participants[1]?.ultracode ?? null,
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
          <h2 className="font-headline-lg text-headline-lg text-on-surface">Sessions</h2>
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
              // FIXED frame (user 2026-08-25): the size is the viewport-clamped
              // constant, never the content — adding participants scrolls the
              // roster list instead of growing the dialog, and removing them
              // does not shrink it. The min() clamp keeps both edges on-screen
              // for short/narrow windows (a fixed+translated box cannot be
              // scrolled back into view by the page).
              "fixed left-1/2 top-1/2 z-50 flex h-[min(760px,90vh)] w-[min(1100px,92vw)] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden",
              "rounded-lg border border-outline-variant bg-surface-container p-5 shadow-2xl focus:outline-none",
            )}
          >
            <div className="mb-4 flex shrink-0 items-center justify-between">
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
            {/* Two columns (ideas.md 2026-08-24): the session's SCOPE — title,
                project, repo, worktree — on the left; the ROSTER on the right.
                Inside the fixed frame the body is the flexible middle: each
                column scrolls itself on md+, and below md the whole body
                scrolls as one stack (a per-list scroller inside an
                auto-height column would constrain nothing there). */}
            <div className="min-h-0 flex-1 gap-x-6 overflow-x-hidden max-md:space-y-4 max-md:overflow-y-auto md:flex">
              <div className="min-h-0 space-y-4 overflow-x-hidden md:w-[340px] md:shrink-0 md:overflow-y-auto md:pr-1">
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
                  className={selectClass}
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
                <div className="flex items-center justify-between gap-2 rounded border border-outline-variant bg-surface-container-lowest px-3 py-1.5">
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
              <div className="flex min-h-0 min-w-0 flex-col md:flex-1">
                <div className="mb-1 flex shrink-0 items-center justify-between">
                  <span className="font-label-caps text-label-caps text-on-surface-variant">
                    Participants
                  </span>
                  <span className="font-code-sm text-code-sm text-on-surface-variant">
                    {participants.length} of {MAX_PARTICIPANTS}
                  </span>
                </div>
                {/* The one scroller on md+: overflow lands here, never on the
                    frame — the footer stays reachable at any roster size. No
                    flex-1: the list takes its natural height and SHRINKS
                    (min-h-0) when the column runs out, so the add-button and
                    hints sit right under the cards instead of pinned to the
                    column floor. */}
                <div className="flex min-h-0 flex-col gap-2 overflow-x-hidden md:overflow-y-auto md:pr-1">
                  {participants.map((row, index) => (
                    <div
                      key={row.key}
                      className="rounded-md border border-outline-variant bg-surface p-2"
                    >
                      <div className="mb-1 flex items-center justify-between gap-2">
                        <span className="shrink-0 font-code-sm text-code-sm text-on-surface-variant">
                          Participant {index + 1}
                        </span>
                        {/* rc3 D20: the colour this participant's byline takes,
                            folded into the header line so the controls fit one
                            row. "Rotate" is the DEFAULT rather than an absence
                            — the rotation already guarantees no two
                            participants of one session share a hue, so a pick
                            is a preference, not a fix. Swatches rather than a
                            select: the thing being chosen is a colour, and its
                            name is the label of last resort. */}
                        <span className="flex items-center gap-1">
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
                            <RescanIcon size={10} className="mx-auto" />
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
                          {participants.length > 1 && (
                            <button
                              type="button"
                              aria-label={`Remove participant ${index + 1}`}
                              onClick={() =>
                                setParticipants((rows) =>
                                  rows.filter((_, i) => i !== index),
                                )
                              }
                              className="ml-1 text-on-surface-variant transition-colors hover:text-on-surface"
                            >
                              ×
                            </button>
                          )}
                        </span>
                      </div>
                      {/* One line for the dropdowns (user 2026-08-25) — the
                          fixed frame is wide enough that stacking them read as
                          wasted space. Collapses to 2×2 below md. */}
                      <div className="grid grid-cols-2 gap-2 md:grid-cols-4">
                        <label className="block">
                          <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
                            Role
                          </span>
                          <select
                            aria-label={`Participant ${index + 1} role`}
                            value={row.roleId ?? ""}
                            onChange={(e) => {
                              const roleId = e.target.value
                                ? Number(e.target.value)
                                : null;
                              const incoming = roles.find(
                                (r) => r.id === roleId,
                              );
                              // An ultracode pick cannot follow the row to a
                              // role that can't take it (no edit_files): the
                              // option would grey out while the row still
                              // shipped {effort:"xhigh", ultracode:true} — a
                              // posture the user never picked for THAT role.
                              // Reset the whole pick to Default, not just the
                              // flag, so no phantom xhigh survives either.
                              const dropUltracode =
                                row.ultracode === true &&
                                !incoming?.capabilities.includes(EDIT_FILES);
                              patchParticipant(index, {
                                roleId,
                                ...(dropUltracode
                                  ? { effort: null, ultracode: null }
                                  : {}),
                              });
                            }}
                            className={selectClass}
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
                            className={selectClass}
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
                            the role and model it applies to. One select since
                            no-inherit (2026-08-25): ultracode is a choice in
                            it, not a sibling checkbox — a pick decomposes to
                            the stored pair via `pickToFields`, and picking any
                            concrete level explicitly clears a role-default
                            ultracode. */}
                        <label className="block">
                          <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
                            Effort
                          </span>
                          <select
                            aria-label={`Participant ${index + 1} effort`}
                            value={
                              row.ultracode === true
                                ? ULTRACODE
                                : row.effort ?? ""
                            }
                            onChange={(e) =>
                              patchParticipant(index, pickToFields(e.target.value))
                            }
                            className={selectClass}
                          >
                            <option value="">
                              Default ({defaultEffortFor(row.roleId)})
                            </option>
                            {EFFORT_LEVELS.map((v) => (
                              <option key={v} value={v}>
                                {v}
                              </option>
                            ))}
                            {/* Ultracode rides in on `--settings`, which spawn
                                injects only for a role holding `edit_files` —
                                offering it to any other role would be a choice
                                that changes nothing. Capability, not role name:
                                bot-hq does not know what a role means. */}
                            <option
                              value={ULTRACODE}
                              disabled={
                                !roleAt(row)?.capabilities.includes(EDIT_FILES)
                              }
                            >
                              {ULTRACODE}
                            </option>
                          </select>
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
                      </div>
                    </div>
                  ))}
                </div>
                <div className="shrink-0">
                {participants.length < MAX_PARTICIPANTS && (
                  <button
                    type="button"
                    onClick={() =>
                      setParticipants((rows) => [
                        ...rows,
                        { ...emptyParticipant(), roleId: nextUnusedRoleId(rows) },
                      ])
                    }
                    className="mt-2 font-code-sm text-code-sm text-primary transition-colors hover:underline"
                  >
                    + Add participant
                  </button>
                )}
                <p className="mt-1 font-code-sm text-code-sm text-on-surface-variant">
                  Row order sets each participant's turn slot. Effort applies to
                  this session only — Default uses the role's configured effort
                  (Settings → Roles).
                </p>
                {/* rc3 D11. Advisory, never blocking: it names a gap in what
                    the roster's ticked boxes allow, and the user decides
                    whether that is what they meant. */}
                {rosterWarning && (
                  <p
                    role="status"
                    className="mt-2 rounded border border-warning/50 bg-warning/15 px-3 py-2 font-code-sm text-code-sm text-warning"
                  >
                    <WarnIcon size={12} className="mr-1 inline-block align-[-2px]" />
                    {rosterWarning}
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
              </div>
            </div>
            {createError && (
              <p className="mt-4 shrink-0 rounded border border-error/40 bg-error-container/20 px-3 py-2 font-code-sm text-code-sm text-on-error-container">
                Create failed: {createError}
              </p>
            )}
            <div className="mt-5 flex shrink-0 justify-end gap-2">
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
        <Skeleton
          className="grid grid-cols-1 gap-gutter md:grid-cols-2 xl:grid-cols-3"
          rowClassName="h-40 rounded-lg border border-outline-variant bg-surface"
        />
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
            <div
              key={s.id}
              draggable
              data-testid={`tile-wrap-${s.id}`}
              onDragStart={(e) => {
                setDragId(s.id);
                e.dataTransfer.effectAllowed = "move";
                // Some WebViews cancel the drag without payload data.
                e.dataTransfer.setData("text/plain", s.id);
              }}
              onDragEnd={() => {
                setDragId(null);
                setDropTarget(null);
              }}
              onDragOver={(e) => {
                // preventDefault is what makes this a legal drop target.
                e.preventDefault();
                if (dragId && dragId !== s.id) setDropTarget(s.id);
              }}
              onDragLeave={() =>
                setDropTarget((t) => (t === s.id ? null : t))
              }
              onDrop={(e) => {
                e.preventDefault();
                handleTileDrop(s.id);
              }}
              className={cn(
                "rounded-lg",
                dragId === s.id && "opacity-60",
                dropTarget === s.id && "ring-2 ring-primary/60",
              )}
            >
              <SessionTileLoader
                session={s}
                pendingCount={pendingBySession[s.id] ?? 0}
              />
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
