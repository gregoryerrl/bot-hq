import { useEffect, useRef, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { useQueryClient } from "@tanstack/react-query";
import { useTauriQuery, useTauriMutation, errorMessage } from "../hooks/useInvoke";
import { useTauriEvent } from "../hooks/useTauriEvent";
import { useHealthStore } from "../stores/health";
import { useActivityStore } from "../stores/activity";
import { HealthDot, RouterHealthDot } from "../components/HealthDot";
import { useStickyScroll } from "../hooks/useStickyScroll";
import { useDragResize } from "../hooks/useDragResize";
import { useChatStore } from "../stores/chat";
import { ChatInput } from "../components/ChatInput";
import { ChatMessage } from "../components/ChatMessage";
import { DocumentPane } from "../components/DocumentPane";
import { type Phase } from "../components/PhasePill";
import { SessionFindingsBanner } from "../components/SessionFindingsBanner";
import { SessionPolicyPanel } from "./SessionPolicyPanel";
import { cn } from "../lib/cn";
import type {
  AgentMessage,
  AppError,
  SessionInfo,
} from "../lib/bindings";
import { Button } from "../components/ui/Button";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { GearIcon } from "../components/icons";
import { SubTabButton } from "../components/SubTabButton";
import { invoke } from "@tauri-apps/api/core";

const PHASE_NAMES: Phase[] = ["investigate", "plan", "apply", "verify"];

/** Session-container subtabs. Workspace = chat + IPAV documents (the
 * original session view); Context and Terminal land with this arc. */
type SessionTab = "workspace" | "context" | "terminal";

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

// Stable reference so zustand selector doesn't return a fresh array per call
// (would trigger infinite re-renders via Object.is).
const EMPTY_MESSAGES: AgentMessage[] = [];

export function SessionView() {
  const { sessionId = "" } = useParams<{ sessionId: string }>();
  const navigate = useNavigate();

  const { data: session, error: sessionError } = useTauriQuery<
    SessionInfo | null
  >("get_session", { sessionId });

  // Respawn agents on mount. Idempotent — `ensure_session_started` is a no-op
  // if Brian/Rain are already running. Reads `brian_claude_session_id` /
  // `rain_claude_session_id` from the session row + passes `--resume <uuid>`
  // so the agents come back with full memory.
  const respawn = useTauriMutation<void, { sessionId: string }>(
    "respawn_session",
  );
  const [respawnError, setRespawnError] = useState<AppError | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);

  // Active subtab. All three panels stay MOUNTED — inactive ones are `hidden`
  // (display:none) — so chat scroll position, CL editor state, and the xterm
  // buffer survive tab switches (same keep-mounted convention as Settings).
  const [tab, setTab] = useState<SessionTab>("workspace");

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
  const [closing, setClosing] = useState(false);
  const [closeError, setCloseError] = useState<string | null>(null);
  const [showCloseConfirm, setShowCloseConfirm] = useState(false);
  // B4: # of uncommitted entries in the session's working tree, probed when the
  // user opens the close-confirm so the dialog can warn the work will be kept.
  const [dirtyCount, setDirtyCount] = useState(0);
  // B2: live agent health for this session (drives the header dots).
  const health = useHealthStore((s) => s.bySession[sessionId]);
  const routerAlive = useHealthStore((s) => s.routerBySession[sessionId]);
  const activity = useActivityStore((s) => s.bySession[sessionId]);
  // Per-agent busy flags for the chat-input turn-status line ("Brian is
  // working… / Rain is reviewing…"). Parallel to `activity`.
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

  const { data: initialMsgs = [], isLoading: messagesLoading } = useTauriQuery<
    AgentMessage[]
  >(
    "get_session_messages",
    { sessionId, sinceId: null },
    { enabled: !!sessionId },
  );

  const messages = useChatStore(
    (s) => s.messages[sessionId] ?? EMPTY_MESSAGES,
  );
  const setMessages = useChatStore((s) => s.setMessages);
  const applyBatch = useChatStore((s) => s.applyBatch);
  const clearChat = useChatStore((s) => s.clear);

  useEffect(() => {
    if (initialMsgs.length > 0) {
      setMessages(sessionId, initialMsgs);
    }
  }, [initialMsgs, sessionId, setMessages]);

  useTauriEvent<AgentMessage[]>(
    "agent:messages:batch",
    (batch) => {
      const forSession = batch.filter((m) => m.session_id === sessionId);
      if (forSession.length > 0) applyBatch(forSession);
    },
    [sessionId, applyBatch],
  );

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

  // Auto-scroll on new messages when user is at-bottom; show "↓ N new" jump
  // button when they've scrolled up.
  const { ref: scrollRef, stuck, scrollToBottom } =
    useStickyScroll<HTMLDivElement>([messages.length]);

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
                  Phase: <span className="text-on-surface">{phase}</span>
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
            {session.brian_model_at_spawn && (
              <>
                <span className="mx-2 text-outline-variant">·</span>
                <span
                  className="text-on-surface-variant"
                  title="Live agent health (models are in Session Settings)"
                >
                  Brian <HealthDot health={health?.brian} name="Brian" />
                  {session.rain_enabled && (
                    <>
                      <span className="mx-1.5 text-outline-variant">·</span>
                      Rain <HealthDot health={health?.rain} name="Rain" />
                      {routerAlive === false && (
                        <>
                          <span className="mx-1.5 text-outline-variant">·</span>
                          <span className="text-error">
                            router <RouterHealthDot alive={routerAlive} />
                          </span>
                        </>
                      )}
                    </>
                  )}
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
            title="Force-close session — ends Brian + Rain and archives it"
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
            session and stops Brian + Rain immediately — their subprocesses are
            killed regardless of any in-flight work. The session moves to
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
          onClick={() => setTab("context")}
        >
          Context
        </SubTabButton>
        <SubTabButton
          active={tab === "terminal"}
          onClick={() => setTab("terminal")}
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
          {/*
           * Single scroll boundary: the scroll container IS the positioning
           * context for the floating "Jump to latest" button. The button is
           * absolutely positioned inside the scroll container itself with
           * `position: sticky`-equivalent layout via inset offsets, kept
           * out of the document flow so it doesn't push messages.
           */}
          <div
            ref={scrollRef}
            className="relative min-h-0 flex-1 overflow-y-auto overflow-x-hidden px-4 py-3"
          >
            {messagesLoading && messages.length === 0 ? (
              <MessagesSkeleton />
            ) : messages.length === 0 ? (
              <p className="font-body-md text-body-md text-on-surface-variant">No messages yet…</p>
            ) : (
              messages.map((m, i) => (
                <ChatMessage
                  key={m.id}
                  message={m}
                  groupedWithPrev={
                    i > 0 &&
                    m.kind !== "phase_change" &&
                    messages[i - 1].kind !== "phase_change" &&
                    messages[i - 1].author === m.author
                  }
                />
              ))
            )}
            {!stuck && messages.length > 0 && (
              <div className="pointer-events-none sticky bottom-0 flex justify-end pr-1 pt-2">
                <button
                  onClick={scrollToBottom}
                  className={cn(
                    "pointer-events-auto inline-flex items-center gap-1 rounded-full",
                    "border border-outline-variant bg-surface-container-highest px-3 py-1 font-code-sm text-code-sm text-on-surface shadow-lg",
                    "hover:border-primary hover:text-on-surface transition-colors",
                  )}
                >
                  ↓ Jump to latest
                </button>
              </div>
            )}
          </div>

          <div className="border-t border-outline-variant">
            {/* key remounts the input per session so the draft seed (a lazy
                initializer) re-runs — without it, switching sessions would
                carry session A's text into session B. */}
            <ChatInput
              key={sessionId}
              draftKey={`bothq:draft:${sessionId}`}
              placeholder="Broadcast to Brian + Rain…"
              activity={activity}
              busy={busy}
              onSend={async (text) => {
                await invoke("broadcast_message", { sessionId, text });
              }}
              onCancel={async () => {
                await invoke("cancel_session_turn", { sessionId });
              }}
            />
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
        <TabPlaceholder
          label="Context"
          hint="Project-scoped Context Library — files and proposals for this session's project."
        />
      </div>

      <div
        role="tabpanel"
        aria-label="Terminal"
        className={cn("min-h-0 flex-1", tab !== "terminal" && "hidden")}
      >
        <TabPlaceholder
          label="Terminal"
          hint="Session terminal — agents run commands here, you watch (and type)."
        />
      </div>

      <SessionPolicyPanel
        session={session}
        sessionId={sessionId}
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
      />
    </div>
  );
}

/** Interim empty-state for subtabs whose content lands later in this arc. */
function TabPlaceholder({ label, hint }: { label: string; hint: string }) {
  return (
    <div className="flex h-full items-center justify-center p-6">
      <div className="text-center">
        <p className="font-body-md text-body-md text-on-surface">{label}</p>
        <p className="mt-1 font-code-sm text-code-sm text-on-surface-variant">
          {hint} Not wired up yet.
        </p>
      </div>
    </div>
  );
}

function MessagesSkeleton() {
  return (
    <div className="space-y-4">
      {[0, 1, 2].map((i) => (
        <div key={i} className="space-y-2">
          <div className="h-3 w-12 animate-pulse rounded bg-surface-container-high" />
          <div className="h-3 w-3/4 animate-pulse rounded bg-surface-container-high" />
          <div className="h-3 w-1/2 animate-pulse rounded bg-surface-container-high" />
        </div>
      ))}
    </div>
  );
}
