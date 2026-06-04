import {
  QueryClient,
  QueryClientProvider,
  useQueryClient,
} from "@tanstack/react-query";
import { useCallback, useState, type ReactNode } from "react";
import { useTauriEvent } from "./hooks/useTauriEvent";

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
const TRAY_KEYS = [
  "list_pending_tray",
  "list_pending_choices",
  "list_session_tray",
] as const;
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
  const onClose = useCallback(() => invalidate(CLOSE_KEYS), [invalidate]);

  useTauriEvent("session:pending_choice", onTray, [onTray]);
  useTauriEvent("session:choice_resolved", onTray, [onTray]);
  useTauriEvent("session:awaiting_user", onTray, [onTray]);
  useTauriEvent("session:phase_changed", onPhase, [onPhase]);
  useTauriEvent("session:doc_changed", onDoc, [onDoc]);
  useTauriEvent("session:closed", onClose, [onClose]);

  return null;
}
