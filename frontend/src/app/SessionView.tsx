import { useEffect, useRef, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { useTauriQuery, useTauriMutation, errorMessage } from "../hooks/useInvoke";
import { useTauriEvent } from "../hooks/useTauriEvent";
import { useStickyScroll } from "../hooks/useStickyScroll";
import { useChatStore } from "../stores/chat";
import { ChatInput } from "../components/ChatInput";
import { ChatMessage } from "../components/ChatMessage";
import { DocumentPane } from "../components/DocumentPane";
import { type Phase } from "../components/PhasePill";
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
import { invoke } from "@tauri-apps/api/core";

interface PhaseChangedPayload {
  session_id: string;
  agent: string;
  target: string;
}

const PHASE_NAMES: Phase[] = ["investigate", "plan", "apply", "verify"];

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

  // Resizable chat/document split. `leftPct` is the chat pane's width as a % of
  // the container; the rest goes to the DocumentPane. Seeded from localStorage
  // and clamped to [25,75] so neither pane can be dragged away entirely.
  const splitContainerRef = useRef<HTMLDivElement>(null);
  const [leftPct, setLeftPct] = useState<number>(() => {
    const saved = Number(localStorage.getItem("bothq:split:leftPct"));
    return Number.isFinite(saved) && saved >= 25 && saved <= 75 ? saved : 60;
  });
  const onSplitHandleDown = (e: React.MouseEvent) => {
    e.preventDefault();
    const container = splitContainerRef.current;
    if (!container) return;
    let latest = leftPct;
    const onMove = (ev: MouseEvent) => {
      const rect = container.getBoundingClientRect();
      const pct = ((ev.clientX - rect.left) / rect.width) * 100;
      latest = Math.min(75, Math.max(25, pct));
      setLeftPct(latest);
    };
    const onUp = () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      localStorage.setItem("bothq:split:leftPct", String(Math.round(latest)));
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
  };
  const [closing, setClosing] = useState(false);
  const [closeError, setCloseError] = useState<string | null>(null);
  const [showCloseConfirm, setShowCloseConfirm] = useState(false);

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

  // IPAV phase indicator. Initial value comes from `get_session_phase`
  // (in-memory on `CoreAppState`); subsequent updates arrive via the
  // `session:phase_changed` Tauri event fired by the bridge subscriber.
  const { data: initialPhase } = useTauriQuery<string | null>(
    "get_session_phase",
    { sessionId },
    { enabled: !!sessionId },
  );
  const [phase, setPhase] = useState<Phase | null>(null);
  useEffect(() => {
    setPhase(normalizePhase(initialPhase));
  }, [initialPhase]);
  useTauriEvent<PhaseChangedPayload>(
    "session:phase_changed",
    (payload) => {
      if (payload.session_id !== sessionId) return;
      const next = normalizePhase(payload.target);
      if (next) setPhase(next);
    },
    [sessionId],
  );
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
      <div className="p-6 text-sm text-on-surface-variant">
        {sessionError ? (
          <>
            <p className="mb-2 text-on-error-container">
              Failed to load session: {sessionError.message}
            </p>
            <p className="text-xs text-on-surface-variant">id: {sessionId}</p>
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
    <div ref={splitContainerRef} className="flex h-full">
      <section
        style={{ flexBasis: `${leftPct}%` }}
        className="flex h-full min-h-0 min-w-0 shrink-0 grow-0 flex-col border-r border-outline-variant"
      >
        <header className="flex items-center justify-between border-b border-outline-variant px-4 py-3">
          <div className="min-w-0">
            <h1 className="truncate text-base font-semibold tracking-tight">
              {session.title}
            </h1>
            <p className="text-xs text-on-surface-variant">
              <Link to="/" className="hover:text-on-surface">
                ← Dashboard
              </Link>
              <span className="mx-2 text-outline-variant">·</span>
              <code className="font-mono text-[0.65rem] text-on-surface-variant">
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
              onClick={() => setShowCloseConfirm(true)}
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
            </>
          }
          confirmLabel="Force-close"
          cancelLabel="Keep open"
          confirmVariant="danger"
          onConfirm={onCloseSession}
          onCancel={() => setShowCloseConfirm(false)}
        />

        {respawnError && (
          <div className="border-b border-outline-variant bg-error-container/30 px-4 py-2 text-xs text-on-error-container">
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
          <div className="border-b border-outline-variant bg-error-container/30 px-4 py-2 text-xs text-on-error-container">
            <span className="font-semibold">Close failed:</span> {closeError}
            <button
              className="ml-2 underline hover:text-error"
              onClick={() => setCloseError(null)}
            >
              dismiss
            </button>
          </div>
        )}

        {/*
         * Single scroll boundary: the scroll container IS the positioning
         * context for the floating "Jump to latest" button. The button is
         * absolutely positioned inside the scroll container itself with
         * `position: sticky`-equivalent layout via inset offsets, kept
         * out of the document flow so it doesn't push messages.
         */}
        <div
          ref={scrollRef}
          className="relative min-h-0 flex-1 overflow-auto px-4 py-3"
        >
          {messagesLoading && messages.length === 0 ? (
            <MessagesSkeleton />
          ) : messages.length === 0 ? (
            <p className="text-sm text-on-surface-variant">No messages yet…</p>
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
                  "border border-outline-variant bg-surface-container-highest px-3 py-1 text-xs text-on-surface shadow-lg",
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
            onSend={async (text) => {
              await invoke("broadcast_message", { sessionId, text });
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

      <SessionPolicyPanel
        sessionId={sessionId}
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
      />
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

