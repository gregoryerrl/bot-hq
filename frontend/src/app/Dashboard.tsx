import { useMemo, useState } from "react";
import { useTauriQuery, useTauriMutation } from "../hooks/useInvoke";
import { SessionTile } from "../components/SessionTile";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import type { PendingChoiceView, SessionInfo } from "../lib/bindings";

/**
 * Thin wrapper that drives the per-session phase query, so `SessionTile`
 * stays pure presentational (test-friendly without a QueryClient). Each
 * loader is its own hook call — fine for the typical bot-hq session count
 * (< 20). React Query dedupes by `["get_session_phase", { sessionId }]`.
 */
function SessionTileLoader({
  session,
  pendingChoices,
}: {
  session: SessionInfo;
  pendingChoices: PendingChoiceView[];
}) {
  const { data: phase = null } = useTauriQuery<string | null>(
    "get_session_phase",
    { sessionId: session.id },
    { refetchInterval: 5_000 },
  );
  return (
    <SessionTile
      session={session}
      pendingChoices={pendingChoices}
      phase={phase}
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

  const { data: pending = [] } = useTauriQuery<PendingChoiceView[]>(
    "list_pending_choices",
    {},
    { refetchInterval: 5_000 },
  );

  const createSession = useTauriMutation<
    SessionInfo,
    {
      id: string;
      title: string;
      repoPath: string | null;
      project: string | null;
    }
  >("create_session");

  const pendingBySession = useMemo(() => {
    const acc: Record<string, PendingChoiceView[]> = {};
    for (const p of pending) {
      (acc[p.session_id] = acc[p.session_id] ?? []).push(p);
    }
    return acc;
  }, [pending]);

  const [creating, setCreating] = useState(false);
  const [title, setTitle] = useState("");
  const [repoPath, setRepoPath] = useState("");
  const [filter, setFilter] = useState("");

  // Case-insensitive substring filter on session title. In-memory so no
  // debounce needed — the list isn't a paginated query.
  const filteredSessions = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return sessions;
    return sessions.filter((s) => s.title.toLowerCase().includes(q));
  }, [sessions, filter]);

  const handleCreate = async () => {
    if (!title.trim()) return;
    const id = `s-${crypto.randomUUID().slice(0, 8)}`;
    await createSession.mutateAsync({
      id,
      title: title.trim(),
      repoPath: repoPath.trim() || null,
      project: null,
    });
    setTitle("");
    setRepoPath("");
    setCreating(false);
    refetch();
  };

  return (
    <div className="mx-auto h-full max-w-6xl overflow-auto px-6 py-6">
      <div className="mb-6 flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold tracking-tight">Sessions</h1>
          <p className="mt-1 text-xs text-neutral-500">
            {filter.trim()
              ? `${filteredSessions.length} of ${sessions.length} match`
              : `${sessions.length} active`}
          </p>
        </div>
        <Button variant="primary" onClick={() => setCreating(!creating)}>
          {creating ? "Cancel" : "+ New session"}
        </Button>
      </div>
      {creating && (
        <div className="mb-6 space-y-2 rounded-lg border border-default bg-surface p-3">
          <div className="flex gap-2">
            <Input
              autoFocus
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="Session title (e.g., refactor auth flow)"
              onKeyDown={(e) => {
                if (e.key === "Enter") handleCreate();
              }}
            />
            <Button
              variant="primary"
              onClick={handleCreate}
              disabled={!title.trim() || createSession.isPending}
            >
              Create
            </Button>
          </div>
          <Input
            value={repoPath}
            onChange={(e) => setRepoPath(e.target.value)}
            placeholder="Working repo path (optional — enables git diff in Apply tab)"
            onKeyDown={(e) => {
              if (e.key === "Enter") handleCreate();
            }}
          />
        </div>
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
              className="absolute inset-y-0 right-0 flex w-8 items-center justify-center text-neutral-500 hover:text-neutral-100"
            >
              ×
            </button>
          )}
        </div>
      )}
      {error && (
        <div className="mb-6 rounded-lg border border-red-500/40 bg-red-950/30 px-4 py-3">
          <p className="text-sm text-red-200">
            Failed to load sessions: {error.message}
          </p>
          <button
            onClick={() => refetch()}
            className="mt-1 text-xs text-red-300 underline hover:text-red-100"
          >
            Retry
          </button>
        </div>
      )}
      {isLoading ? (
        <div className="grid grid-cols-1 gap-gutter md:grid-cols-2 xl:grid-cols-3">
          {[0, 1, 2].map((i) => (
            <div
              key={i}
              className="h-40 animate-pulse rounded-lg border border-outline-variant bg-surface"
            />
          ))}
        </div>
      ) : sessions.length === 0 ? (
        <div className="rounded-lg border border-dashed border-outline-variant p-10 text-center">
          <p className="text-sm text-on-surface">
            No active sessions yet.
          </p>
          <p className="mt-1 text-xs text-on-surface-variant">
            Click <b>+ New session</b> to spawn a Brian + Rain duo on a scope.
          </p>
        </div>
      ) : filteredSessions.length === 0 ? (
        <div className="rounded-lg border border-dashed border-outline-variant p-10 text-center">
          <p className="text-sm text-on-surface-variant">
            No sessions match <code className="font-mono">{filter.trim()}</code>.
          </p>
        </div>
      ) : (
        <div className="grid grid-cols-1 gap-gutter md:grid-cols-2 xl:grid-cols-3">
          {filteredSessions.map((s) => (
            <SessionTileLoader
              key={s.id}
              session={s}
              pendingChoices={pendingBySession[s.id] ?? []}
            />
          ))}
        </div>
      )}
    </div>
  );
}
