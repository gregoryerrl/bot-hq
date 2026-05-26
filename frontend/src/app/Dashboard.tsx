import { useState } from "react";
import { useTauriQuery, useTauriMutation } from "../hooks/useInvoke";
import { SessionTile } from "../components/SessionTile";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import type { SessionInfo } from "../lib/bindings";

export function Dashboard() {
  const {
    data: sessions = [],
    refetch,
    isLoading,
  } = useTauriQuery<SessionInfo[]>("list_sessions");

  const createSession = useTauriMutation<
    SessionInfo,
    {
      id: string;
      title: string;
      repoPath: string | null;
      project: string | null;
    }
  >("create_session");

  const [creating, setCreating] = useState(false);
  const [title, setTitle] = useState("");

  const handleCreate = async () => {
    if (!title.trim()) return;
    const id = `s-${crypto.randomUUID().slice(0, 8)}`;
    await createSession.mutateAsync({
      id,
      title: title.trim(),
      repoPath: null,
      project: null,
    });
    setTitle("");
    setCreating(false);
    refetch();
  };

  return (
    <div className="mx-auto h-full max-w-6xl overflow-auto px-6 py-6">
      <div className="mb-6 flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold tracking-tight">Sessions</h1>
          <p className="mt-1 text-xs text-neutral-500">
            {sessions.length} active
          </p>
        </div>
        <Button variant="primary" onClick={() => setCreating(!creating)}>
          {creating ? "Cancel" : "+ New session"}
        </Button>
      </div>
      {creating && (
        <div className="mb-6 flex gap-2 rounded-lg border border-default bg-surface p-3">
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
      )}
      {isLoading ? (
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {[0, 1, 2].map((i) => (
            <div
              key={i}
              className="h-24 animate-pulse rounded-lg border border-default bg-surface"
            />
          ))}
        </div>
      ) : sessions.length === 0 ? (
        <div className="rounded-lg border border-dashed border-default p-16 text-center">
          <p className="text-sm text-neutral-300">
            No active sessions yet.
          </p>
          <p className="mt-1 text-xs text-neutral-500">
            Click <b>+ New session</b> to spawn a Brian + Rain duo on a scope.
          </p>
        </div>
      ) : (
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
          {sessions.map((s) => (
            <SessionTile key={s.id} session={s} />
          ))}
        </div>
      )}
    </div>
  );
}
