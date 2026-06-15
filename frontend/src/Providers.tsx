import {
  QueryClient,
  QueryClientProvider,
  useQueryClient,
} from "@tanstack/react-query";
import { useCallback, useState, type ReactNode } from "react";
import { useTauriEvent } from "./hooks/useTauriEvent";
import { useHealthStore, type AgentHealth } from "./stores/health";

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
// are infrequent. NOTE: `cl_read_file` is deliberately EXCLUDED — EditorPane seeds
// its editable `draft` once on mount and never re-syncs from the query, so
// invalidating an open file wouldn't update the textarea (sticky draft) and would
// only flip a clean editor into a spurious "dirty" state. Live open-file refresh
// needs an editor-side draft re-seed — a separate follow-up.
const CL_KEYS = ["cl_index_search", "list_projects", "cl_folder_search"] as const;
// Working-tree freshness: the fs watcher fires `session:worktree_changed` when a
// file changes inside a live session's repo, so the Apply-tab diff re-runs live
// (not just on a phase/doc write).
const WORKTREE_KEYS = ["compute_apply_diff"] as const;

/**
 * Event-driven cache invalidation: each backend `session:*` event invalidates
 * only the query families it can affect (see the key maps above). Renders
 * nothing. `agent:messages:batch` is intentionally excluded — the chat consumes
 * it directly, and a chat message doesn't change other views.
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
  const setHealth = useHealthStore((s) => s.setHealth);
  const clearHealth = useHealthStore((s) => s.clearSession);
  const onClose = useCallback(
    (p: { session_id: string }) => {
      invalidate(CLOSE_KEYS);
      clearHealth(p.session_id);
    },
    [invalidate, clearHealth],
  );
  const onHealth = useCallback(
    (p: { session_id: string; agent: string; health: string }) => {
      setHealth(p.session_id, p.agent, p.health as AgentHealth);
    },
    [setHealth],
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
      ]),
    [invalidate],
  );

  useTauriEvent("session:pending_choice", onTray, [onTray]);
  useTauriEvent("session:choice_resolved", onTray, [onTray]);
  useTauriEvent("session:awaiting_user", onTray, [onTray]);
  useTauriEvent("session:halt_cleared", onTray, [onTray]);
  useTauriEvent("session:phase_changed", onPhase, [onPhase]);
  useTauriEvent("session:doc_changed", onDoc, [onDoc]);
  useTauriEvent("cl:changed", onCl, [onCl]);
  useTauriEvent("session:worktree_changed", onWorktree, [onWorktree]);
  useTauriEvent("session:closed", onClose, [onClose]);
  useTauriEvent("session:agent_health", onHealth, [onHealth]);
  useTauriEvent("session:resync", onResync, [onResync]);

  return null;
}
