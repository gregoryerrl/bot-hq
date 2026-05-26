import { useState } from "react";
import { useTauriQuery, useTauriMutation } from "../hooks/useInvoke";
import { SessionTile } from "../components/SessionTile";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import type { SessionInfo } from "../lib/bindings";

export function Dashboard() {
  const { data: sessions = [], refetch, isLoading } = useTauriQuery<SessionInfo[]>(
    "list_sessions",
  );

  const createSession = useTauriMutation<SessionInfo, {
    id: string;
    title: string;
    repo_path: string | null;
    project: string | null;
  }>("create_session");

  const [creating, setCreating] = useState(false);
  const [title, setTitle] = useState("");

  const handleCreate = async () => {
    if (!title.trim()) return;
    const id = `s-${crypto.randomUUID().slice(0, 8)}`;
    await createSession.mutateAsync({
      id,
      title: title.trim(),
      repo_path: null,
      project: null,
    });
    setTitle("");
    setCreating(false);
    refetch();
  };

  return (
    <div className="mx-auto max-w-6xl px-6 py-6">
      <div className="mb-6 flex items-center justify-between">
        <h1 className="text-xl font-semibold">Sessions</h1>
        <Button variant="primary" onClick={() => setCreating(!creating)}>
          {creating ? "Cancel" : "+ New session"}
        </Button>
      </div>
      {creating && (
        <div className="mb-6 flex gap-2 rounded-lg border border-neutral-800 bg-neutral-900/50 p-3">
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
        <p className="text-sm text-neutral-500">Loading…</p>
      ) : sessions.length === 0 ? (
        <div className="rounded-lg border border-dashed border-neutral-800 p-12 text-center">
          <p className="text-sm text-neutral-400">
            No active sessions. Click <b>+ New session</b> to start.
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
