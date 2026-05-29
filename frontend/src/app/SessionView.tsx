import { useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import { useTauriQuery, useTauriMutation } from "../hooks/useInvoke";
import { useTauriEvent } from "../hooks/useTauriEvent";
import { useStickyScroll } from "../hooks/useStickyScroll";
import { useScreenshotCapture } from "../hooks/useScreenshotCapture";
import { useChatStore } from "../stores/chat";
import { ChatInput } from "../components/ChatInput";
import { ChatMessage } from "../components/ChatMessage";
import { ChoicePrompt } from "../components/ChoicePrompt";
import { DocumentPane } from "../components/DocumentPane";
import { PhasePillRow, type Phase } from "../components/PhasePill";
import { cn } from "../lib/cn";
import type {
  AgentMessage,
  AppError,
  PendingChoiceView,
  PermissionActionView,
  SessionInfo,
  SessionPermissionsView,
} from "../lib/bindings";
import { Button } from "../components/ui/Button";
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
  const {
    capture: captureScreenshot,
    pending: screenshotPending,
    error: screenshotError,
    dismissError: dismissScreenshotError,
  } = useScreenshotCapture(sessionId);
  // Track which (choiceId, option) is mid-resolve so the clicked button can
  // show "…" and ALL options for that choice disable until the invoke
  // settles. Banner naturally disappears on the next list_pending_choices
  // refresh after resolve_choice lands.
  const [resolvingChoice, setResolvingChoice] = useState<
    Map<string, string>
  >(new Map());

  const handleResolveChoice = async (choiceId: string, picked: string) => {
    setResolvingChoice((s) => new Map(s).set(choiceId, picked));
    try {
      await invoke("resolve_choice", { choiceId, picked });
    } finally {
      setResolvingChoice((s) => {
        const next = new Map(s);
        next.delete(choiceId);
        return next;
      });
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

  const { data: pendingChoices = [] } = useTauriQuery<PendingChoiceView[]>(
    "list_pending_choices",
  );
  const choicesForSession = pendingChoices.filter(
    (c) => c.session_id === sessionId,
  );

  // Per-session commit + push grants. The agents check these before running
  // git commit / git push. Two states the user can toggle from the header:
  // `none` (default; agents must call request_approval) and `all_branches`
  // (blanket grant for the session). Per-branch specific grants are issued
  // by agents via request_approval and arrive as ask_user_choice — no UI
  // surface needed here for that path.
  const { data: perms, refetch: refetchPerms } =
    useTauriQuery<SessionPermissionsView>(
      "list_session_permissions",
      { sessionId },
      { enabled: !!sessionId },
    );
  const togglePermission = async (action: PermissionActionView) => {
    const current = perms?.[action]?.kind;
    try {
      if (current === "all_branches") {
        await invoke("revoke_session_permission", { sessionId, action });
      } else {
        await invoke("grant_session_permission", {
          sessionId,
          action,
          scope: { kind: "all_branches" },
        });
      }
      refetchPerms();
    } catch (e) {
      // eslint-disable-next-line no-console
      console.warn(`toggle ${action} grant failed`, e);
    }
  };

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

  // Auto-scroll on new messages when user is at-bottom; show "↓ N new" jump
  // button when they've scrolled up.
  const { ref: scrollRef, stuck, scrollToBottom } =
    useStickyScroll<HTMLDivElement>([messages.length]);

  if (!session) {
    return (
      <div className="p-6 text-sm text-neutral-500">
        {sessionError ? (
          <>
            <p className="mb-2 text-red-300">
              Failed to load session: {sessionError.message}
            </p>
            <p className="text-xs text-neutral-500">id: {sessionId}</p>
          </>
        ) : (
          <>Session not found.</>
        )}{" "}
        <Link to="/" className="text-blue-400 underline">
          Back to dashboard
        </Link>
      </div>
    );
  }

  return (
    <div className="grid h-full grid-cols-[3fr_2fr] grid-rows-1">
      <section className="flex h-full min-h-0 min-w-0 flex-col border-r border-default">
        <header className="flex items-center justify-between border-b border-default px-4 py-3">
          <div className="min-w-0">
            <h1 className="truncate text-base font-semibold tracking-tight">
              {session.title}
            </h1>
            <p className="text-xs text-neutral-500">
              <Link to="/" className="hover:text-neutral-300">
                ← Dashboard
              </Link>
              <span className="mx-2 text-neutral-700">·</span>
              <code className="font-mono text-[0.65rem] text-neutral-600">
                {sessionId.slice(0, 8)}
              </code>
              {phase && (
                <>
                  <span className="mx-2 text-neutral-700">·</span>
                  <span className="text-neutral-400">
                    Phase: <span className="text-neutral-200">{phase}</span>
                  </span>
                </>
              )}
            </p>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            <GrantPill
              label="commit"
              granted={perms?.commit.kind === "all_branches"}
              specific={perms?.commit.kind === "specific"}
              onToggle={() => togglePermission("commit")}
            />
            <GrantPill
              label="push"
              granted={perms?.push.kind === "all_branches"}
              specific={perms?.push.kind === "specific"}
              onToggle={() => togglePermission("push")}
            />
            <PhasePillRow
              selected={phase ?? "investigate"}
              onSelect={(next) => {
                setPhase(next);
                invoke("advance_session_phase", {
                  sessionId,
                  target: next,
                }).catch((e) => {
                  // Roll back optimistic state; the next event refresh
                  // will reconcile if the bridge state actually advanced.
                  setPhase(normalizePhase(initialPhase));
                  // eslint-disable-next-line no-console
                  console.warn("advance_session_phase failed", e);
                });
              }}
            />
            <Button
              variant="ghost"
              size="sm"
              title="Capture the bot-hq window and share with Brian + Rain"
              disabled={screenshotPending}
              onClick={captureScreenshot}
            >
              {screenshotPending ? "…" : "📸"}
            </Button>
          </div>
        </header>

        {respawnError && (
          <div className="border-b border-default bg-red-950/30 px-4 py-2 text-xs text-red-200">
            <span className="font-semibold">Agent spawn failed:</span>{" "}
            {respawnError.message}{" "}
            <button
              className="ml-2 underline"
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

        {screenshotError && (
          <div className="border-b border-default bg-red-950/30 px-4 py-2 text-xs text-red-200">
            <span className="font-semibold">Screenshot failed:</span>{" "}
            {screenshotError}
            <button
              className="ml-2 underline"
              onClick={dismissScreenshotError}
            >
              dismiss
            </button>
          </div>
        )}

        {choicesForSession.length > 0 && (
          <div className="border-b border-default bg-surface-container px-4 py-3">
            <div className="mb-2 flex items-center gap-2 font-label-caps text-label-caps text-secondary">
              <span className="inline-block h-2 w-2 rounded-full bg-secondary motion-safe:animate-pulse" />
              {choicesForSession.length === 1
                ? "Awaiting your input"
                : `Awaiting your input · ${choicesForSession.length}`}
            </div>
            <div className="space-y-2">
              {choicesForSession.map((choice) => (
                <ChoicePrompt
                  key={choice.choice_id}
                  choice={choice}
                  pendingOption={resolvingChoice.get(choice.choice_id)}
                  onResolve={handleResolveChoice}
                />
              ))}
            </div>
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
            <p className="text-sm text-neutral-500">No messages yet…</p>
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
                  "border border-default bg-overlay px-3 py-1 text-xs text-neutral-200 shadow-lg",
                  "hover:border-accent hover:text-white transition-colors",
                )}
              >
                ↓ Jump to latest
              </button>
            </div>
          )}
        </div>

        <div className="border-t border-default">
          <ChatInput
            placeholder="Broadcast to Brian + Rain…"
            onSend={async (text) => {
              await invoke("broadcast_message", { sessionId, text });
            }}
          />
        </div>
      </section>

      <DocumentPane sessionId={sessionId} sessionPhase={phase} />
    </div>
  );
}

function MessagesSkeleton() {
  return (
    <div className="space-y-4">
      {[0, 1, 2].map((i) => (
        <div key={i} className="space-y-2">
          <div className="h-3 w-12 animate-pulse rounded bg-elevated" />
          <div className="h-3 w-3/4 animate-pulse rounded bg-elevated" />
          <div className="h-3 w-1/2 animate-pulse rounded bg-elevated" />
        </div>
      ))}
    </div>
  );
}

// Compact commit/push grant toggle. Three visual states keyed off the
// underlying SessionPermissions kind:
//   - granted=true   → blanket all_branches grant (primary-tinted)
//   - specific=true  → per-branch grant (informational; click still toggles
//                      via the granted path)
//   - neither        → no grant (default; outline only)
// Click cycles between none ↔ all_branches via the parent's onToggle.
function GrantPill({
  label,
  granted,
  specific,
  onToggle,
}: {
  label: string;
  granted: boolean;
  specific: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onToggle}
      title={
        granted
          ? `Click to revoke session-wide ${label} permission`
          : specific
            ? `${label}: per-branch grant active. Click to upgrade to all-branches.`
            : `Click to grant ${label} for all branches this session`
      }
      className={
        "inline-flex items-center gap-1 rounded border px-2 py-1 font-label-caps text-label-caps transition-colors " +
        (granted
          ? "border-primary bg-primary/15 text-primary"
          : specific
            ? "border-tertiary/60 text-tertiary"
            : "border-outline/40 text-on-surface-variant hover:border-outline")
      }
    >
      <span aria-hidden>{granted ? "●" : specific ? "◐" : "○"}</span>
      {label}
    </button>
  );
}
