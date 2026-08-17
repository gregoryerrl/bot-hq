import { useEffect, useMemo, useRef, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { useQueryClient } from "@tanstack/react-query";
import { useTauriQuery, useTauriMutation, errorMessage } from "../hooks/useInvoke";
import { useTauriEvent } from "../hooks/useTauriEvent";
import { useHealthStore } from "../stores/health";
import { useActivityStore } from "../stores/activity";
import { HealthDot } from "../components/HealthDot";
import { ContextMeter } from "../components/ContextMeter";
import { useContextStore } from "../stores/context";
import { useDragResize } from "../hooks/useDragResize";
import { useChatStore } from "../stores/chat";
import { ChatInput, draftKeyFor } from "../components/ChatInput";
import { HaltBanner, isApproval, type SessionHalt, type TrayRow } from "../components/HaltBanner";
import { useTrayStaging, stagedFor } from "../stores/trayStaging";
import { ApprovalGate } from "../components/ApprovalGate";
import { ChatPane } from "../components/ChatPane";
import { DocumentPane } from "../components/DocumentPane";
import { type Phase } from "../components/PhasePill";
import { SessionFindingsBanner } from "../components/SessionFindingsBanner";
import { SessionPolicyPanel } from "./SessionPolicyPanel";
import { cn } from "../lib/cn";
import {
  ATTENTION_IDLE_LABEL,
  ATTENTION_IDLE_TOOLTIP,
  ATTENTION_IDLE_UNFLAGGED,
} from "../lib/attention";
import {
  authorLabel,
  participantLabel,
  participantRuntime,
  useParticipantLabels,
  type ParticipantView,
} from "../lib/participants";
import { FileViewerDialog } from "../components/FileViewerDialog";
import { SpawnBadge } from "../components/SpawnBadge";
import type { AppError, ResolveResult, SessionInfo } from "../lib/bindings";
import { Button } from "../components/ui/Button";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { GearIcon } from "../components/icons";
import { SubTabButton } from "../components/SubTabButton";
import { SessionContextTab } from "./SessionContextTab";
import { SessionTerminalTab } from "./SessionTerminalTab";
import { invoke } from "@tauri-apps/api/core";

const PHASE_NAMES: Phase[] = ["investigate", "plan", "apply", "verify"];

/** Session-container subtabs. Workspace = chat + IPAV documents (the
 * original session view); Context and Terminal land with this arc. */
type SessionTab = "workspace" | "context" | "terminal";

/** Hand mirror of the Rust `ParticipantSystemPrompt` (rc3 P1) — the raw-invoke
 * convention `WorkspaceFile` follows, so `bindings.ts` (which is `@ts-nocheck`
 * and regenerates only at app launch) is not what tsc checks here. */
interface ParticipantSystemPrompt {
  slug: string;
  content: string | null;
  bytes: number;
  unavailable: string | null;
}

/** Hand mirror of the Rust `ContextReadingView` (rc3 P7). */
interface ContextReadingView {
  model: string | null;
  used_tokens: number | null;
  reported_window: number | null;
  verdict: string;
  created_at: string;
}

/** One recorded reading as a fixed-width line. Operands are printed exactly as
 *  recorded — a missing one renders as `—`, never as a substituted figure, so a
 *  provider that sent no window stays visibly different from one that did. */
function readingLine(r: ContextReadingView): string {
  const used = r.used_tokens?.toLocaleString() ?? "—";
  const window = r.reported_window?.toLocaleString() ?? "—";
  const pct =
    r.used_tokens !== null && r.reported_window
      ? ` (${Math.floor((r.used_tokens / r.reported_window) * 100)}%)`
      : "";
  return `${r.created_at}  ${r.verdict.padEnd(19)}${used} / ${window}${pct}${
    r.model ? `  ${r.model}` : ""
  }`;
}

function normalizePhase(raw: string | null | undefined): Phase | null {
  if (!raw) return null;
  const lower = raw.toLowerCase();
  // Accept either single-letter chips ("I"/"P"/"A"/"V") or full names.
  switch (lower) {
    case "i":
    case "investigate":
      return "investigate";
    case "p":
    case "plan":
      return "plan";
    case "a":
    case "apply":
      return "apply";
    case "v":
    case "verify":
      return "verify";
    default:
      return PHASE_NAMES.includes(lower as Phase) ? (lower as Phase) : null;
  }
}

export function SessionView() {
  const { sessionId = "" } = useParams<{ sessionId: string }>();
  const navigate = useNavigate();

  const { data: session, error: sessionError } = useTauriQuery<
    SessionInfo | null
  >("get_session", { sessionId });

  // Who is in this session, in turn order — the header roster (rc3 D10) — plus
  // the key → `ROLE · Model` index every author-keyed surface below resolves
  // through (the turn-status line, and the roster row itself).
  const { participants, labels, hues } = useParticipantLabels(sessionId);

  // rc3 D30: why the session stopped, rendered above the input box. Same query
  // key the tray tab uses, so answering anywhere invalidates both.
  const { data: trayRows = [] } = useTauriQuery<TrayRow[]>("list_session_tray", {
    sessionId,
  });
  // rc3 D35: the halt is SESSION state — its own slot, its own query, nothing
  // to do with the tray. Invalidated by the same awaiting/halt-cleared events.
  const { data: sessionHalt = null } = useTauriQuery<SessionHalt | null>(
    "get_session_halt",
    { sessionId },
  );
  // rc3 D33: approvals are not parkable — they take the input slot. The gate
  // shows rows[0], and `tray_entries_for_session` is already `ORDER BY id ASC`,
  // so the row that has been blocking longest is the one asked first.
  const pendingApprovals = useMemo(
    () => trayRows.filter((r) => r.status === "pending" && isApproval(r)),
    [trayRows],
  );
  // rc3 D34: tray picks staged to travel with the next Send. Pruned against
  // the pending rows at Send time — a row answered or superseded elsewhere
  // must not be re-answered by a stale stage.
  const stagedMap = useTrayStaging((s) => stagedFor(s.staged, sessionId));
  const clearStaged = useTrayStaging((s) => s.clear);
  const stagedCount = useMemo(
    () =>
      trayRows.filter(
        (r) =>
          r.status === "pending" && !isApproval(r) && stagedMap[r.choice_id],
      ).length,
    [trayRows, stagedMap],
  );

  // Respawn agents on mount. Idempotent — `ensure_session_started` is a no-op
  // if the session's agents are already running. Reads each agent's stored
  // claude session id from the session row + passes `--resume <uuid>` so they
  // come back with full memory.
  const respawn = useTauriMutation<void, { sessionId: string }>(
    "respawn_session",
  );
  // rc3 round-4: the phase-write path the harness already promised agents.
  // Same `AppState::advance_phase` the agent-side tool reaches, so the user's
  // path and the participants' cannot drift.
  const advancePhase = useTauriMutation<
    void,
    { sessionId: string; target: string }
  >("advance_session_phase");
  const [respawnError, setRespawnError] = useState<AppError | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);

  // Active subtab. All three panels stay MOUNTED — inactive ones are `hidden`
  // (display:none) — so chat scroll position, CL editor state, and the xterm
  // buffer survive tab switches (same keep-mounted convention as Settings).
  // Context/Terminal content additionally mounts only on FIRST activation, so
  // opening a session doesn't fire their queries (or spawn a PTY) unseen.
  const [tab, setTab] = useState<SessionTab>("workspace");
  const [contextMounted, setContextMounted] = useState(false);
  const [terminalMounted, setTerminalMounted] = useState(false);

  // Resizable chat/document split. `leftPct` is the chat pane's width as a % of
  // the container; the rest goes to the DocumentPane. Seeded from localStorage
  // and clamped to [25,75] so neither pane can be dragged away entirely.
  const splitContainerRef = useRef<HTMLDivElement>(null);
  const [leftPct, setLeftPct] = useState<number>(() => {
    const saved = Number(localStorage.getItem("bothq:split:leftPct"));
    return Number.isFinite(saved) && saved >= 25 && saved <= 75 ? saved : 60;
  });
  const onSplitHandleDown = useDragResize({
    containerRef: splitContainerRef,
    value: leftPct,
    setValue: setLeftPct,
    storageKey: "bothq:split:leftPct",
    compute: (ev, rect) =>
      Math.min(75, Math.max(25, ((ev.clientX - rect.left) / rect.width) * 100)),
  });
  // rc3 P1: the composed system prompt of whichever participant chip was
  // clicked. Held as resolved TEXT rather than a slug so a slow read can't land
  // under a different participant's heading.
  // A file a gate names (`--body-file /tmp/x.md`), opened in the same
  // full-screen viewer the prompt view uses; `read_workspace_file` reads it.
  const [viewFile, setViewFile] = useState<{
    sessionId: string;
    path: string;
  } | null>(null);
  const [promptView, setPromptView] = useState<{
    title: string;
    text: string;
    note?: string;
  } | null>(null);

  /** Read what this participant was actually told, and show it. */
  const openComposedPrompt = async (p: ParticipantView) => {
    const title = `${participantLabel(p)} — composed system prompt`;
    try {
      const view = await invoke<ParticipantSystemPrompt>(
        "get_participant_system_prompt",
        { sessionId, slug: p.slug },
      );
      setPromptView({
        title,
        // An unavailable prompt still opens the dialog, carrying its reason —
        // a blank pane would read as "the prompt is empty", which is the
        // opposite of what this view exists to show.
        text: view.content ?? view.unavailable ?? "",
        note: view.content
          ? `${view.bytes.toLocaleString()} bytes, composed at spawn. bot-hq APPENDS this to ` +
            `claude-code's own system prompt, so what you see here is bot-hq's portion only.`
          : undefined,
      });
    } catch (e) {
      setPromptView({ title, text: errorMessage(e) });
    }
  };

  /** What this participant's context window was doing — including after the
   *  session is closed, which is when it matters (rc3 P7). */
  const openContextHistory = async (p: ParticipantView) => {
    const title = `${participantLabel(p)} — context readings`;
    try {
      const rows = await invoke<ContextReadingView[]>(
        "list_participant_context_readings",
        { sessionId, slug: p.slug },
      );
      setPromptView({
        title,
        text: rows.length
          ? rows.map(readingLine).join("\n")
          : "No readings recorded for this participant yet — it has not " +
            "completed a turn since bot-hq started recording them.",
        note:
          "One row per completed turn, oldest first. `no_window` means the " +
          "provider reported no context window, so no meter was possible; " +
          "`implausible_window` means the prompt exceeded the window it was " +
          "accepted into, so the figure is not trusted for display. Nothing " +
          "here is substituted from the model's configured context window.",
      });
    } catch (e) {
      setPromptView({ title, text: errorMessage(e) });
    }
  };

  const [closing, setClosing] = useState(false);
  const [closeError, setCloseError] = useState<string | null>(null);
  const [showCloseConfirm, setShowCloseConfirm] = useState(false);
  // B4: # of uncommitted entries in the session's working tree, probed when the
  // user opens the close-confirm so the dialog can warn the work will be kept.
  const [dirtyCount, setDirtyCount] = useState(0);
  // B2: live agent health for this session (drives the header dots).
  const health = useHealthStore((s) => s.bySession[sessionId]);
  const attention = useHealthStore((s) => s.attentionBySession[sessionId]);
  const agentContext = useContextStore((s) => s.bySession[sessionId]);
  const activity = useActivityStore((s) => s.bySession[sessionId]);
  // Per-agent busy flags for the chat-input turn-status line. Parallel to
  // `activity`; keyed by participant slug, rendered as ROLE · Model.
  const busy = useActivityStore((s) => s.busyBySession[sessionId]);

  // Inline title rename. `editingTitle === null` = display mode; a string =
  // the in-progress edit. Commit on Enter/blur, cancel on Escape.
  const queryClient = useQueryClient();
  const [editingTitle, setEditingTitle] = useState<string | null>(null);
  const [renameError, setRenameError] = useState<string | null>(null);
  const commitRename = async () => {
    const next = (editingTitle ?? "").trim();
    setEditingTitle(null);
    if (!next || !session || next === session.title) return;
    try {
      await invoke("rename_session", { sessionId, title: next });
      queryClient.invalidateQueries({ queryKey: ["get_session"] });
      queryClient.invalidateQueries({ queryKey: ["list_sessions"] });
    } catch (e) {
      setRenameError(errorMessage(e));
    }
  };

  // The actual force-close, fired only after the user confirms in the dialog.
  // The backend kill is unconditional — it does not wait for an in-flight turn.
  const onCloseSession = async () => {
    setShowCloseConfirm(false);
    setClosing(true);
    setCloseError(null);
    try {
      // On success the `session:closed` event (listener below) navigates back
      // to the dashboard, so this component unmounts — no need to reset state.
      await invoke("close_session", { sessionId, archive: true });
    } catch (e) {
      setCloseError(errorMessage(e));
      setClosing(false);
    }
  };

  // B4: probe for uncommitted work before opening the close-confirm, so the
  // dialog can warn the user it'll be kept (not committed). Best-effort — the
  // dialog opens regardless if the probe fails.
  const onCloseClick = async () => {
    setDirtyCount(0);
    try {
      const d = await invoke<{ dirty_count: number }>("check_session_dirty", {
        sessionId,
      });
      setDirtyCount(d.dirty_count);
    } catch {
      // ignore — show the dialog without the warning
    }
    setShowCloseConfirm(true);
  };

  useEffect(() => {
    if (!sessionId) return;
    setRespawnError(null);
    respawn.mutate(
      { sessionId },
      { onError: (err) => setRespawnError(err) },
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId]);

  const clearChat = useChatStore((s) => s.clear);

  // IPAV phase indicator. `get_session_phase` (in-memory on `CoreAppState`) is
  // invalidated by the global `session:phase_changed` listener in Providers, so
  // a phase transition refetches it and the effect below re-syncs local state.
  const { data: initialPhase } = useTauriQuery<string | null>(
    "get_session_phase",
    { sessionId },
    { enabled: !!sessionId },
  );
  const [phase, setPhase] = useState<Phase | null>(null);
  useEffect(() => {
    setPhase(normalizePhase(initialPhase));
  }, [initialPhase]);
  // When THIS session finishes closing, purge its messages from the store
  // (closed sessions otherwise accumulate forever) and leave its now-dead
  // view for the dashboard instead of stranding the user inside it.
  useTauriEvent<{ session_id: string }>(
    "session:closed",
    (payload) => {
      if (payload.session_id !== sessionId) return;
      clearChat(sessionId);
      navigate("/");
    },
    [sessionId, navigate, clearChat],
  );
  // The Stage toggle (2026-08-15): the backend holds the staged response;
  // this query is the toggle's source of truth (reload-safe). Delivery fires
  // `session:stage_delivered` — bump the tick (clears the composer draft),
  // drop the consumed tray picks, and refetch everything the delivery moved.
  const { data: stagedResp = null, refetch: refetchStaged } = useTauriQuery<{
    text: string;
    picks: { choice_id: string; picked: string }[];
  } | null>("get_staged_response", { sessionId });
  const [deliveredTick, setDeliveredTick] = useState(0);
  useTauriEvent<{ session_id: string }>(
    "session:stage_delivered",
    (payload) => {
      if (payload.session_id !== sessionId) return;
      setDeliveredTick((t) => t + 1);
      // **Clear the PERSISTED draft too, not just the live box.**
      //
      // `deliveredTick` is a transient edge, and only a MOUNTED ChatInput can
      // observe it. ChatInput is unmounted whenever an approval gate holds the
      // input slot (`pendingApprovals.length > 0` below), so a staged message
      // delivered during a gate bumps the tick with nobody watching: on
      // remount `prevTickRef` seeds to the current tick, sees no change, and
      // the box re-seeds itself from this key — the delivered message, back in
      // the box, needing a manual clear.
      //
      // Removing the key here makes the clear survive that, because it is
      // durable state rather than an event nobody heard. The tick still does
      // the in-memory clear for the common mounted case; this is the half that
      // cannot be missed.
      localStorage.removeItem(draftKeyFor(sessionId));
      clearStaged(sessionId);
      // Drop the delivered content NOW rather than waiting for the refetch.
      // The refetch is async, so until it lands `stagedResp` still describes a
      // message the backend has already sent — and a stale non-null staged
      // response beside a freshly cleared box is what refilled the composer.
      // ChatInput guards this too; closing the window here means the guard is
      // a backstop rather than the only thing standing between the user and
      // clearing the box by hand.
      queryClient.setQueryData(["get_staged_response", { sessionId }], null);
      void refetchStaged();
      void queryClient.invalidateQueries({
        queryKey: ["list_session_tray", { sessionId }],
      });
    },
    [sessionId, clearStaged, refetchStaged, queryClient],
  );
  // Picks staged AFTER the message was staged must ride too: re-stage with
  // the same text whenever the pick set changes while staged, so the backend
  // snapshot always equals what the tray shows as staged.
  //
  // **A DELIVERY also changes the pick set, and must not re-stage.** Delivery
  // calls `clearStaged`, which empties `stagedMap` and moves this counter — so
  // this effect ran, saw the pick set shrink to zero beside a `stagedResp` the
  // refetch had not yet nulled, and re-staged the message that had just been
  // sent. That is why the composer would not stay clear: the box was not
  // refilling itself, the message was genuinely being put back. Guarded on the
  // tick rather than on `stagedResp` alone, so it does not depend on the
  // delivery handler's cache write winning a race with this render.
  const stagedPickCount = Object.keys(stagedMap).length;
  const deliveredAtRef = useRef(deliveredTick);
  useEffect(() => {
    const justDelivered = deliveredAtRef.current !== deliveredTick;
    deliveredAtRef.current = deliveredTick;
    if (justDelivered) return;
    if (!stagedResp) return;
    const current = trayRows
      .filter(
        (r) =>
          r.status === "pending" && !isApproval(r) && stagedMap[r.choice_id],
      )
      .map((r) => ({ choice_id: r.choice_id, picked: stagedMap[r.choice_id]! }));
    if (
      current.length !== stagedResp.picks.length ||
      current.some(
        (p, i) =>
          stagedResp.picks[i]?.choice_id !== p.choice_id ||
          stagedResp.picks[i]?.picked !== p.picked,
      )
    ) {
      void invoke("stage_user_response", {
        sessionId,
        text: stagedResp.text,
        picks: current,
      }).then(() => refetchStaged());
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [stagedPickCount, deliveredTick]);

  if (!session) {
    return (
      <div className="p-6 font-body-md text-body-md text-on-surface-variant">
        {sessionError ? (
          <>
            <p className="mb-2 text-on-error-container">
              Failed to load session: {sessionError.message}
            </p>
            <p className="font-code-sm text-code-sm text-on-surface-variant">id: {sessionId}</p>
          </>
        ) : (
          <>Session not found.</>
        )}{" "}
        <Link to="/" className="text-tertiary underline">
          Back to dashboard
        </Link>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <header className="flex items-center justify-between border-b border-outline-variant px-4 py-3">
        <div className="min-w-0">
          {editingTitle === null ? (
            <h1 className="group flex items-center gap-1 truncate font-headline-md text-headline-md tracking-tight">
              <span className="truncate">{session.title}</span>
              <button
                type="button"
                aria-label="Rename session"
                title="Rename session"
                onClick={() => {
                  setRenameError(null);
                  setEditingTitle(session.title);
                }}
                className="shrink-0 px-1 font-code-sm text-code-sm text-on-surface-variant opacity-0 transition-opacity hover:text-on-surface focus:opacity-100 group-hover:opacity-100"
              >
                ✎
              </button>
            </h1>
          ) : (
            <input
              autoFocus
              value={editingTitle}
              onChange={(e) => setEditingTitle(e.target.value)}
              onBlur={commitRename}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  commitRename();
                } else if (e.key === "Escape") {
                  e.preventDefault();
                  setEditingTitle(null);
                }
              }}
              aria-label="Session title"
              className="w-full max-w-sm rounded border border-outline-variant bg-surface px-2 py-0.5 font-headline-md text-headline-md tracking-tight text-on-surface focus:border-primary focus:outline-none"
            />
          )}
          <p className="font-code-sm text-code-sm text-on-surface-variant">
            <Link to="/" className="hover:text-on-surface">
              ← Dashboard
            </Link>
            <span className="mx-2 text-outline-variant">·</span>
            <code className="font-code-sm text-code-sm text-on-surface-variant">
              {sessionId.slice(0, 8)}
            </code>
            {phase && (
              <>
                <span className="mx-2 text-outline-variant">·</span>
                <span className="text-on-surface-variant">
                  Phase:{" "}
                  {/* **The user's own hand on the phase**, and until round 4 it
                      did not exist: no Tauri command wrote the phase and this
                      was plain text — while `protocol.rs` told every agent "the
                      ring stops until the user advances the chip". An agent
                      calling `request_phase_advance` could only ever be
                      declined.

                      It lives HERE and not on `SessionPhaseChip`, which is the
                      obvious-looking home: that chip's only render site is the
                      dashboard tile, whose whole surface is `onClick={open}` →
                      navigate, so a button inside it would both advance and
                      navigate away (and nest interactives). It also puts the
                      control in the list rather than in the session — and the
                      escape valve for a stalled phase is needed while you are
                      IN one. */}
                  <select
                    aria-label="Session phase"
                    className="rounded border border-outline-variant bg-surface-container px-1 py-0.5 text-on-surface"
                    value={phase}
                    onChange={(e) => {
                      void advancePhase
                        .mutateAsync({
                          sessionId,
                          target: e.target.value,
                        })
                        .catch(() => {});
                    }}
                  >
                    {PHASE_NAMES.map((name) => (
                      <option key={name} value={name}>
                        {name}
                      </option>
                    ))}
                  </select>
                </span>
              </>
            )}
            <SessionFindingsBanner sessionId={sessionId} />
            {session.base_repo_path && (
              <>
                <span className="mx-2 text-outline-variant">·</span>
                <span
                  className="font-code-sm text-code-sm text-on-surface-variant"
                  title={`Isolated worktree of ${session.base_repo_path} — work lands on branch bothq/${sessionId}`}
                >
                  ⎇ bothq/{sessionId}
                </span>
              </>
            )}
            {/* rc3 D10 + the roster the user asked for: who is IN this session,
                as ROLE · Model, legible while it runs. The live complaint was
                that a duplicate-role roster was invisible until the agents said
                so — this is where it becomes visible. */}
            {participants.map((p) => (
              <span key={p.id}>
                <span className="mx-2 text-outline-variant">·</span>
                <span
                  className={cn(
                    "text-on-surface-variant",
                    !p.enabled && "opacity-50",
                  )}
                  title={
                    p.enabled
                      ? `Participant ${p.turn_position + 1} — ${p.participation_mode}. Live agent health.`
                      : `Participant ${p.turn_position + 1} — not running in this session.`
                  }
                >
                  {/* rc3 P1: the label is the way in to what this participant
                      was actually told. Attribution is structural — the chip
                      you click IS the participant, so a prompt can't be shown
                      under the wrong heading. */}
                  <button
                    type="button"
                    onClick={() => void openComposedPrompt(p)}
                    title="View the composed system prompt this participant spawned with"
                    className="underline decoration-dotted underline-offset-2 transition-colors hover:text-on-surface"
                  >
                    {participantLabel(p)}
                  </button>
                  {/* CONFIGURATION, so it sits beside the label rather than
                      inside the runtime pair below: what this participant was
                      spawned with is a fact about the spawn, while the dot and
                      the meter are facts about the process. */}
                  <SpawnBadge participant={p} />{" "}
                  {/* Both stores hold TWO key spaces: the live
                      `session:agent_health` / `session:agent_context` events
                      key by the participant's slug, while the mount backfill
                      unpacks the slot-shaped `get_session_runtime` pair. A bare
                      `[p.slug]` read the first and missed the second, so every
                      dot and meter went blank after a restart until the next
                      transition — `participantRuntime` spans both. It needs the
                      whole roster because a slot is a place IN it: the backend
                      numbers slots over the SPAWNABLE rows, so a disabled or
                      on-demand row shifts every slot after it. */}
                  <HealthDot
                    health={participantRuntime(health, participants, p)}
                    name={participantLabel(p)}
                  />
                  <ContextMeter
                    context={participantRuntime(agentContext, participants, p)}
                    name={participantLabel(p)}
                    onOpenHistory={() => void openContextHistory(p)}
                  />
                </span>
              </span>
            ))}
            {attention === ATTENTION_IDLE_UNFLAGGED && (
              <>
                <span className="mx-2 text-outline-variant">·</span>
                <span
                  className="rounded border border-warning/50 bg-warning/15 px-1.5 py-0.5 font-label-caps text-label-caps text-warning"
                  title={ATTENTION_IDLE_TOOLTIP}
                >
                  {ATTENTION_IDLE_LABEL}
                </span>
              </>
            )}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Button
            variant="ghost"
            size="sm"
            title="Session settings — policy & push gate"
            aria-label="Session settings"
            onClick={() => setSettingsOpen(true)}
          >
            <GearIcon />
          </Button>
          <Button
            variant="ghost"
            size="sm"
            title="Force-close session — ends its agents and archives it"
            aria-label="Close session"
            disabled={closing}
            onClick={onCloseClick}
          >
            {closing ? "…" : "✕"}
          </Button>
        </div>
      </header>

      <ConfirmDialog
        open={showCloseConfirm}
        title="Force-close session?"
        message={
          <>
            This <strong className="text-on-surface">force-closes</strong> the
            session and stops every participant immediately — their subprocesses
            are killed regardless of any in-flight work. The session moves to
            Settings → Archive; reopening later resumes them via{" "}
            <code className="text-on-surface">--resume</code>.
            {dirtyCount > 0 && (
              <span className="mt-2 block text-warning">
                ⚠️ {dirtyCount} uncommitted change{dirtyCount === 1 ? "" : "s"}{" "}
                in this session's working tree will be kept, not committed.
              </span>
            )}
          </>
        }
        confirmLabel="Force-close"
        cancelLabel="Keep open"
        confirmVariant="danger"
        onConfirm={onCloseSession}
        onCancel={() => setShowCloseConfirm(false)}
      />

      {/* rc3 P1: ~48 KB of standing instruction that nothing could display
          until now. Full-screen, because reading it is the point. */}
      <FileViewerDialog
        target={viewFile}
        inlineTitle={promptView?.title}
        inlineText={promptView?.text}
        inlineNote={promptView?.note}
        onClose={() => {
          setPromptView(null);
          setViewFile(null);
        }}
      />

      {respawnError && (
        <div className="border-b border-outline-variant bg-error-container/30 px-4 py-2 font-code-sm text-code-sm text-on-error-container">
          <span className="font-semibold">Agent spawn failed:</span>{" "}
          {respawnError.message}{" "}
          <button
            className="ml-2 underline hover:text-error"
            onClick={() => {
              setRespawnError(null);
              respawn.mutate(
                { sessionId },
                { onError: (err) => setRespawnError(err) },
              );
            }}
          >
            retry
          </button>
        </div>
      )}

      {closeError && (
        <div className="border-b border-outline-variant bg-error-container/30 px-4 py-2 font-code-sm text-code-sm text-on-error-container">
          <span className="font-semibold">Close failed:</span> {closeError}
          <button
            className="ml-2 underline hover:text-error"
            onClick={() => setCloseError(null)}
          >
            dismiss
          </button>
        </div>
      )}

      {renameError && (
        <div className="border-b border-outline-variant bg-error-container/30 px-4 py-2 font-code-sm text-code-sm text-on-error-container">
          <span className="font-semibold">Rename failed:</span> {renameError}
          <button
            className="ml-2 underline hover:text-error"
            onClick={() => setRenameError(null)}
          >
            dismiss
          </button>
        </div>
      )}

      <nav
        aria-label="Session tabs"
        className="flex shrink-0 items-center border-b border-outline-variant px-2"
      >
        <SubTabButton
          active={tab === "workspace"}
          onClick={() => setTab("workspace")}
        >
          Workspace
        </SubTabButton>
        <SubTabButton
          active={tab === "context"}
          onClick={() => {
            setTab("context");
            setContextMounted(true);
          }}
        >
          Context
        </SubTabButton>
        <SubTabButton
          active={tab === "terminal"}
          onClick={() => {
            setTab("terminal");
            setTerminalMounted(true);
          }}
        >
          Terminal
        </SubTabButton>
      </nav>

      <div
        ref={splitContainerRef}
        role="tabpanel"
        aria-label="Workspace"
        className={cn(
          "flex min-h-0 flex-1",
          tab !== "workspace" && "hidden",
        )}
      >
        <section
          style={{ flexBasis: `${leftPct}%` }}
          className="flex h-full min-h-0 min-w-0 shrink-0 grow-0 flex-col border-r border-outline-variant"
        >
          {/* Chat history + live batches — virtualized, and a render boundary:
              per-batch re-renders stop inside ChatPane instead of re-rendering
              this whole view (header, subtabs, DocumentPane). */}
          <ChatPane sessionId={sessionId} />

          <div className="border-t border-outline-variant">
            {/* rc3 D30: the halt sits ABOVE the box that answers it. */}
            <HaltBanner
              halt={sessionHalt}
              rows={trayRows}
              label={(agent) => authorLabel(agent, labels)}
            />
            {/* rc3 D33: an approval TAKES the input slot. Something is
                synchronously blocked on it — a pre-push hook, a gated command —
                so it is not a card to get to later. Oldest first: a user
                approving a push needs to read that push, not five at once. */}
            {pendingApprovals.length > 0 ? (
              <ApprovalGate
                rows={pendingApprovals}
                label={(agent) => authorLabel(agent, labels)}
                onResolve={async (choiceId, picked, confirmStale = false) => {
                  const res = await invoke<ResolveResult>("resolve_choice", {
                    choiceId,
                    picked,
                    confirmStale,
                  });
                  await queryClient.invalidateQueries({
                    queryKey: ["list_session_tray", { sessionId }],
                  });
                  return res;
                }}
                onCancel={async () => {
                  await invoke("cancel_session_turn", { sessionId });
                }}
                onViewFile={(path) => setViewFile({ sessionId, path })}
              />
            ) : (
            /* key remounts the input per session so the draft seed (a lazy
                initializer) re-runs — without it, switching sessions would
                carry session A's text into session B. */
            <ChatInput
              key={sessionId}
              draftKey={draftKeyFor(sessionId)}
              placeholder="Broadcast to every participant…"
              activity={activity}
              busy={busy}
              // The busy map's keys are slot keys (Providers.tsx unpacks the
              // frozen `slot0_busy`/`slot1_busy` pair); `labels` indexes both
              // key spaces, so one lookup names the participant either way.
              busyLabel={(key) => authorLabel(key, labels)}
              authorHues={hues}
              // rc3 D17: typing `@` offers THIS session's participants and
              // nothing else, which is what makes mentioning a non-participant
              // impossible to express rather than an error to report. Disabled
              // rows are left out — there is no process behind one to summon.
              mentionables={participants
                .filter((p) => p.enabled)
                .map((p) => ({ slug: p.slug, label: participantLabel(p) }))}
              stagedAnswers={stagedCount}
              staged={!!stagedResp}
              stagedText={stagedResp?.text ?? null}
              deliveredTick={deliveredTick}
              onStage={async (text) => {
                // Snapshot the currently staged tray picks with the message —
                // the same set a Send would carry (rc3 D34); the effect above
                // re-stages if the set changes while staged.
                const picks = trayRows
                  .filter(
                    (r) =>
                      r.status === "pending" &&
                      !isApproval(r) &&
                      stagedMap[r.choice_id],
                  )
                  .map((r) => ({
                    choice_id: r.choice_id,
                    picked: stagedMap[r.choice_id]!,
                  }));
                await invoke("stage_user_response", {
                  sessionId,
                  text,
                  picks,
                });
                await refetchStaged();
              }}
              onUnstage={async () => {
                await invoke("unstage_user_response", { sessionId });
                await refetchStaged();
              }}
              onSend={async (text) => {
                // rc3 D34: staged answers travel WITH the message as one user
                // response — answers recorded first, message last, one release.
                const picks = trayRows
                  .filter(
                    (r) =>
                      r.status === "pending" &&
                      !isApproval(r) &&
                      stagedMap[r.choice_id],
                  )
                  .map((r) => ({
                    choice_id: r.choice_id,
                    picked: stagedMap[r.choice_id]!,
                  }));
                if (picks.length > 0) {
                  await invoke("send_user_response", { sessionId, text, picks });
                  clearStaged(sessionId);
                  await queryClient.invalidateQueries({
                    queryKey: ["list_session_tray", { sessionId }],
                  });
                } else {
                  await invoke("broadcast_message", { sessionId, text });
                }
              }}
              onCancel={async () => {
                await invoke("cancel_session_turn", { sessionId });
              }}
              onResume={async () => {
                await invoke("resume_session", { sessionId });
              }}
              onClose={onCloseClick}
            />
            )}
          </div>
        </section>

        <div
          role="separator"
          aria-orientation="vertical"
          aria-label="Resize chat and document panes"
          onMouseDown={onSplitHandleDown}
          className="w-1.5 shrink-0 cursor-col-resize bg-transparent transition-colors hover:bg-primary/40"
        />

        <div className="min-h-0 min-w-0 flex-1">
          <DocumentPane sessionId={sessionId} sessionPhase={phase} />
        </div>
      </div>

      <div
        role="tabpanel"
        aria-label="Context"
        className={cn("min-h-0 flex-1", tab !== "context" && "hidden")}
      >
        {contextMounted && <SessionContextTab sessionId={sessionId} />}
      </div>

      <div
        role="tabpanel"
        aria-label="Terminal"
        className={cn("min-h-0 flex-1", tab !== "terminal" && "hidden")}
      >
        {terminalMounted && (
          <SessionTerminalTab
            sessionId={sessionId}
            active={tab === "terminal"}
          />
        )}
      </div>

      <SessionPolicyPanel
        sessionId={sessionId}
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
      />
    </div>
  );
}
