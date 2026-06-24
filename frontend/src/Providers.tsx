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
import { useHealthStore, type AgentHealth } from "./stores/health";
import { useActivityStore, type SessionActivity } from "./stores/activity";
import { seedRuntimeStores, type SessionRuntime } from "./stores/runtime";

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
// a single choice-resolve during a live duo meant 10-20+ Tauri round-trips
// (incl. `compute_apply_diff` spawning a `git` subprocess). Scope each event to
// only what it can actually change.
const TRAY_KEYS = ["list_pending_tray", "list_session_tray"] as const;
const PHASE_KEYS = [
  "get_session_phase",
  "session_doc_search",
  "compute_apply_diff",
] as const;
const DOC_KEYS = ["session_doc_search", "compute_apply_diff"] as const;
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
// Project registry (register/unregister) and external-driver session creation are
// DB-only changes nothing else refetches, so explicit `app.emit` events drive them.
const PROJECT_KEYS = ["list_projects"] as const;
const SESSION_LIST_KEYS = ["list_sessions"] as const;
// Saved-model registry (upsert/delete) — DB-only, watcher-invisible; the Dashboard
// picker is a cross-view consumer so it needs an explicit event.
const MODEL_KEYS = ["list_models"] as const;
// EYES-sign-off findings — the session-header banner refetches when the bridge
// fires `session:findings_changed` (eyes_flag / disposition_finding / approve_finding).
const FINDINGS_KEYS = ["list_session_findings"] as const;

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
  const onFindings = useCallback(() => invalidate(FINDINGS_KEYS), [invalidate]);
  const setHealth = useHealthStore((s) => s.setHealth);
  const clearHealth = useHealthStore((s) => s.clearSession);
  const setActivity = useActivityStore((s) => s.setActivity);
  const clearActivity = useActivityStore((s) => s.clearSession);
  const onClose = useCallback(
    (p: { session_id: string }) => {
      invalidate(CLOSE_KEYS);
      clearHealth(p.session_id);
      clearActivity(p.session_id);
    },
    [invalidate, clearHealth, clearActivity],
  );
  const onHealth = useCallback(
    (p: { session_id: string; agent: string; health: string }) => {
      setHealth(p.session_id, p.agent, p.health as AgentHealth);
    },
    [setHealth],
  );
  const onActivity = useCallback(
    (p: { session_id: string; state: string }) => {
      setActivity(p.session_id, p.state as SessionActivity);
    },
    [setActivity],
  );
  // Recovery: the backend emits `session:resync` when its broadcast receiver
  // lagged and dropped events — refetch every event-backed query so the UI
  // can't be left stale. This is what lets us drop the fixed-interval safety
  // polls (PendingTray/phase/pending-choices) that previously filled this gap.
  const onResync = useCallback(
    () =>
      invalidate([
        ...TRAY_KEYS,
        ...PHASE_KEYS,
        ...DOC_KEYS,
        ...CLOSE_KEYS,
        ...CL_KEYS,
        ...WORKTREE_KEYS,
        ...PROJECT_KEYS,
        ...MODEL_KEYS,
        ...FINDINGS_KEYS,
      ]),
    [invalidate],
  );

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
  useTauriEvent("session:activity", onActivity, [onActivity]);
  useTauriEvent("session:resync", onResync, [onResync]);

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
      .then((rows) => seedRuntimeStores(rows, setActivity, setHealth))
      .catch(() => {
        // Best-effort: a failed backfill just leaves the stores to the live
        // events (the pre-fix behavior). Never block render.
      });
  }, [setActivity, setHealth]);

  return null;
}
