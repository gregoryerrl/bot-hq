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

/**
 * Event-driven cache invalidation: whenever the backend signals that session
 * state changed, invalidate ALL queries so every view (tray, docs, lists, chat
 * meta) refetches — no polling, no per-view event mapping to forget. Renders
 * nothing. `agent:messages:batch` is intentionally excluded: the chat consumes
 * it directly, and a chat message doesn't change other views, so invalidating
 * everything on each (high-frequency) batch would be wasteful.
 */
function GlobalEventSync() {
  const queryClient = useQueryClient();
  const invalidateAll = useCallback(() => {
    void queryClient.invalidateQueries();
  }, [queryClient]);

  useTauriEvent("session:pending_choice", invalidateAll, [invalidateAll]);
  useTauriEvent("session:choice_resolved", invalidateAll, [invalidateAll]);
  useTauriEvent("session:awaiting_user", invalidateAll, [invalidateAll]);
  useTauriEvent("session:phase_changed", invalidateAll, [invalidateAll]);
  useTauriEvent("session:doc_changed", invalidateAll, [invalidateAll]);
  useTauriEvent("session:closed", invalidateAll, [invalidateAll]);

  return null;
}
