import {
  QueryClient,
  QueryClientProvider,
  useQueryClient,
} from "@tanstack/react-query";
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTauriEvent } from "./hooks/useTauriEvent";
import { draftKeyFor } from "./components/ChatInput";
import { useHealthStore, type AgentHealth } from "./stores/health";
import { useContextStore } from "./stores/context";
import { useChatStore } from "./stores/chat";
import { useActivityStore, type SessionActivity } from "./stores/activity";
import {
  busyBySlot,
  seedRuntimeStores,
  type SessionRuntime,
} from "./stores/runtime";

export function Providers({ children }: { children: ReactNode }) {
  const [queryClient] = useState(
    () =>
      new QueryClient({
        defaultOptions: {
          queries: {
            staleTime: 5_000,
            refetchOnWindowFocus: false,
            retry: 1,
          },
        },
      }),
  );

  return (
    <QueryClientProvider client={queryClient}>
      <GlobalEventSync />
      {children}
    </QueryClientProvider>
  );
}

// Per-event invalidation targets. Query keys are `[command, args]`
// (useInvoke.ts), and `invalidateQueries({ queryKey: [command] })` prefix-
// matches every args variant — so naming the command alone covers all sessions.
// A bare `invalidateQueries()` (no key) refetches EVERY mounted query, which on
// a single choice-resolve during a live multi-participant session meant 10-20+ Tauri round-trips
// (incl. `compute_apply_diff` spawning a `git` subprocess). Scope each event to
// only what it can actually change.
const TRAY_KEYS = [
  "list_pending_tray",
  "list_session_tray",
  // rc3 D35: the halt slot rides the same awaiting/halt-cleared events.
  "get_session_halt",
] as const;
// A phase advance changes only the chip, not doc data (docs refresh via DOC_KEYS
// on a `doc_changed` event) — so `session_doc_search` belongs only in DOC_KEYS.
const PHASE_KEYS = ["get_session_phase"] as const;
const DOC_KEYS = ["session_doc_search"] as const;
const CLOSE_KEYS = [
  "list_sessions",
  "list_closed_sessions",
  "list_pending_tray",
] as const;
// Filesystem-watcher CL freshness. `cl:changed` fires AFTER the watcher re-syncs
// the SQLite index for the changed scope, so refetching here reads fresh rows.
// Invalidation is prefix-based (queryKey is `[command]`), so this refreshes every
// project's CL nav regardless of the event's `project` payload — fine, CL writes
// are infrequent. `cl_read_file` IS included: EditorPane re-seeds its draft from
// the refetched content only when the editor is clean (see ContextLibraryEditor.tsx),
// so an open file live-refreshes on an external change without clobbering unsaved edits.
const CL_KEYS = [
  "cl_index_search",
  "list_projects",
  "cl_folder_search",
  "cl_read_file",
] as const;
// Working-tree freshness: the fs watcher fires `session:worktree_changed` when a
// file changes inside a live session's repo, so the Apply-tab diff re-runs live
// (not just on a phase/doc write).
const WORKTREE_KEYS = ["compute_apply_diff"] as const;
// Project registry (register/unregister) and session creation (the dialog and
// plugin-created sessions alike, `session:created`) are DB-only changes nothing
// else refetches, so explicit `app.emit` events drive them.
const PROJECT_KEYS = ["list_projects"] as const;
const SESSION_LIST_KEYS = ["list_sessions"] as const;
// Saved-model registry (upsert/delete) — DB-only, watcher-invisible; the Dashboard
// picker is a cross-view consumer so it needs an explicit event.
const MODEL_KEYS = ["list_models"] as const;
// EYES-sign-off findings — the session-header banner refetches when the bridge
// fires `session:findings_changed` (eyes_flag / disposition_finding / approve_finding).
const FINDINGS_KEYS = ["list_session_findings"] as const;
// The roster read (round 10). Its rows change at SPAWN (`spawn_knobs_recorded`,
// the effort/ultracode the reconciliation decided) — after a SessionView has
// often already mounted and read them, so the SpawnBadge stayed blank for the
// session. The spawn's first observable event is the agent's health flip, so
// the roster refetches on `session:agent_health` and on the resync sweep.
const ROSTER_KEYS = ["list_session_participants"] as const;
// Plugin registry: install / enable / disable / uninstall / crash all change
// `list_installed_plugins`, which the tab row (Shell) and the manager panel
// share as one cache entry. Round 8: the two components each carried their own
// copies of these listeners; the map is the one place (the `plugin:*` events
// are typed in `src/tauri_events/types.rs`).
const PLUGIN_KEYS = ["list_installed_plugins"] as const;
// Everything a lagged burst could have left stale. `session:resync` fires when
// the backend's broadcast receiver skipped events, and the emitter's watermark
// only recovers a skipped `MessagePersisted` when a LATER row in that session
// arrives — the tail of the burst (the last message before a session goes idle)
// never does. So the chat query is refetched too (round 8): ChatPane re-seeds
// its store from the refetched history. The stage toggle and the session row
// ride the same list for the same reason.
export const RESYNC_KEYS = [
  ...TRAY_KEYS,
  ...PHASE_KEYS,
  ...DOC_KEYS,
  ...CLOSE_KEYS,
  ...CL_KEYS,
  ...WORKTREE_KEYS,
  ...PROJECT_KEYS,
  ...MODEL_KEYS,
  ...FINDINGS_KEYS,
  ...PLUGIN_KEYS,
  ...ROSTER_KEYS,
  "get_session_messages",
  "get_staged_response",
  "get_session",
] as const;

/**
 * Event-driven cache invalidation: each backend `session:*` event invalidates
 * only the query families it can affect (see the key maps above). Renders
 * nothing. `agent:messages:batch` is intentionally excluded from this global
 * map — the chat consumes it directly (SessionView), and the Dashboard consumes
 * it locally for a throttled Quickview refetch (Dashboard.tsx). It changes no
 * OTHER view, so it stays out of the key maps above.
 */
function GlobalEventSync() {
  const queryClient = useQueryClient();
  const invalidate = useCallback(
    (keys: readonly string[]) => {
      for (const key of keys) {
        void queryClient.invalidateQueries({ queryKey: [key] });
      }
    },
    [queryClient],
  );
  const onTray = useCallback(() => invalidate(TRAY_KEYS), [invalidate]);
  const onPhase = useCallback(() => invalidate(PHASE_KEYS), [invalidate]);
  const onDoc = useCallback(() => invalidate(DOC_KEYS), [invalidate]);
  const onCl = useCallback(() => invalidate(CL_KEYS), [invalidate]);
  const onWorktree = useCallback(() => invalidate(WORKTREE_KEYS), [invalidate]);
  const onProject = useCallback(() => invalidate(PROJECT_KEYS), [invalidate]);
  const onSessionCreated = useCallback(
    () => invalidate(SESSION_LIST_KEYS),
    [invalidate],
  );
  const onModel = useCallback(() => invalidate(MODEL_KEYS), [invalidate]);
  const onPlugin = useCallback(() => invalidate(PLUGIN_KEYS), [invalidate]);
  const onFindings = useCallback(() => invalidate(FINDINGS_KEYS), [invalidate]);
  const setHealth = useHealthStore((s) => s.setHealth);
  const setAttention = useHealthStore((s) => s.setAttention);
  const clearHealth = useHealthStore((s) => s.clearSession);
  const setActivity = useActivityStore((s) => s.setActivity);
  const clearActivity = useActivityStore((s) => s.clearSession);
  const setContext = useContextStore((s) => s.setContext);
  const clearContext = useContextStore((s) => s.clearSession);
  const clearChat = useChatStore((s) => s.clear);
  const onClose = useCallback(
    (p: { session_id: string }) => {
      invalidate(CLOSE_KEYS);
      clearHealth(p.session_id);
      clearActivity(p.session_id);
      clearContext(p.session_id);
      // The chat store and the composer draft too — for ANY session (round
      // 8). Only the mounted SessionView cleared them, so a session closed
      // while the user was elsewhere (an agent's `close_session`, another
      // view) kept its whole message array resident for the app's lifetime
      // and left an orphan `bothq:draft:<sid>` key forever.
      clearChat(p.session_id);
      localStorage.removeItem(draftKeyFor(p.session_id));
    },
    [invalidate, clearHealth, clearActivity, clearContext, clearChat],
  );
  const onAgentContext = useCallback(
    (p: {
      session_id: string;
      agent: string;
      used_tokens: number;
      context_window: number;
    }) => {
      setContext(p.session_id, p.agent, {
        usedTokens: p.used_tokens,
        contextWindow: p.context_window,
      });
    },
    [setContext],
  );
  const onHealth = useCallback(
    (p: { session_id: string; agent: string; health: string }) => {
      setHealth(p.session_id, p.agent, p.health as AgentHealth);
      // A health flip is the spawn's first observable edge: re-read the roster
      // so spawn-time columns written after the mount read reach the view.
      invalidate(ROSTER_KEYS);
    },
    [setHealth, invalidate],
  );
  const onAttention = useCallback(
    (p: { session_id: string; state: string | null }) => {
      setAttention(p.session_id, p.state ?? null);
    },
    [setAttention],
  );
  const onActivity = useCallback(
    (p: {
      session_id: string;
      state: string;
      slot0_busy: boolean;
      slot1_busy: boolean;
    }) => {
      // `slot0_busy` / `slot1_busy` name TURN SLOTS 0 and 1, not agents —
      // `src/core/activity.rs` fills them from `slugs.get(0)` / `.get(1)`.
      // They were `brian_busy` / `rain_busy` until the D10 hard retirement, and
      // the names now say what they always meant. Keying them by literal slugs
      // is what made the
      // turn-status line print "brian is working": no rc3 roster has that
      // slug, so the lookup missed and the raw key rendered (rc3 D10).
      //
      // Shared with the `get_session_runtime` backfill so the live event and
      // the mount snapshot cannot key the same session two ways.
      setActivity(p.session_id, p.state as SessionActivity, busyBySlot(p));
    },
    [setActivity],
  );
  // Recovery: the backend emits `session:resync` when its broadcast receiver
  // lagged and dropped events — refetch every event-backed query so the UI
  // can't be left stale. This is what lets us drop the fixed-interval safety
  // polls (PendingTray/phase/pending-choices) that previously filled this gap.
  const onResync = useCallback(() => invalidate(RESYNC_KEYS), [invalidate]);

  // A staged message DELIVERED clears that session's persisted composer draft
  // — for ANY session, mounted or not. `SessionView` clears the live box and
  // the key for the session on screen; a delivery to a session the user is not
  // looking at (the dashboard, another session — they run several) reached no
  // handler, the key kept the sent text, and the box refilled on return
  // (issues.md #3). The composer also drops the key at stage time; this is the
  // half that catches a stage made before that shipped, or from another view.
  const onStageDelivered = useCallback((p: { session_id: string }) => {
    localStorage.removeItem(draftKeyFor(p.session_id));
  }, []);

  useTauriEvent("session:stage_delivered", onStageDelivered, [onStageDelivered]);
  useTauriEvent("session:pending_choice", onTray, [onTray]);
  useTauriEvent("session:choice_resolved", onTray, [onTray]);
  useTauriEvent("session:awaiting_user", onTray, [onTray]);
  useTauriEvent("session:halt_cleared", onTray, [onTray]);
  useTauriEvent("session:phase_changed", onPhase, [onPhase]);
  useTauriEvent("session:doc_changed", onDoc, [onDoc]);
  useTauriEvent("session:findings_changed", onFindings, [onFindings]);
  useTauriEvent("cl:changed", onCl, [onCl]);
  useTauriEvent("session:worktree_changed", onWorktree, [onWorktree]);
  useTauriEvent("project:changed", onProject, [onProject]);
  useTauriEvent("session:created", onSessionCreated, [onSessionCreated]);
  useTauriEvent("model:changed", onModel, [onModel]);
  useTauriEvent("session:closed", onClose, [onClose]);
  useTauriEvent("session:agent_health", onHealth, [onHealth]);
  useTauriEvent("session:agent_context", onAgentContext, [onAgentContext]);
  useTauriEvent("session:attention", onAttention, [onAttention]);
  useTauriEvent("session:activity", onActivity, [onActivity]);
  useTauriEvent("session:resync", onResync, [onResync]);
  useTauriEvent("plugin:state-changed", onPlugin, [onPlugin]);
  useTauriEvent("plugin:uninstalled", onPlugin, [onPlugin]);
  useTauriEvent("plugin:crashed", onPlugin, [onPlugin]);

  // Bug C: backfill the event-driven stores once on mount. The activity/health
  // events fire on transitions and can be missed during the respawn window
  // before these listeners are live, so fetch the current snapshot and seed the
  // stores — otherwise the footer / tiles / input-indicator stay grey until the
  // next transition. The ref guard survives React StrictMode's double-mount.
  const didBackfill = useRef(false);
  useEffect(() => {
    if (didBackfill.current) return;
    didBackfill.current = true;
    invoke<SessionRuntime[]>("get_session_runtime")
      .then((rows) =>
        seedRuntimeStores(
          rows,
          setActivity,
          setHealth,
          setAttention,
        ),
      )
      .catch(() => {
        // Best-effort: a failed backfill just leaves the stores to the live
        // events (the pre-fix behavior). Never block render.
      });
  }, [setActivity, setHealth, setAttention]);

  return null;
}
