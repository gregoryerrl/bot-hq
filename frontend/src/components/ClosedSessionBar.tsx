import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { Button } from "./ui/Button";
import { ErrorBanner } from "./ErrorBanner";
import { errorMessage } from "../hooks/useInvoke";
import { formatTimestamp } from "../lib/time";

/**
 * What a CLOSED session shows where its composer would be (round 10, B4 — the
 * user's pick: "a Reopen button for closed sessions").
 *
 * Viewing a closed session is read-only history now: the SessionView's mount
 * respawn skips a closed row and the backend refuses to spawn one, so nothing
 * comes back to life because somebody clicked through the Archive. The one
 * way to bring the participants back is this button, which clears the row's
 * `closed_at` (and `archived`, so the dashboard lists it again), respawns the
 * roster via `--resume`, and hands the view back to the live composer through
 * a REFETCHED `get_session` read — which this bar asks for itself (round 11,
 * issues.md 2026-08-18): the backend's `session:created` refreshed only the
 * dashboard list, so the closed view stayed on screen after a successful
 * reopen and the user's second click errored "already open".
 */
export function ClosedSessionBar({
  sessionId,
  closedAt,
}: {
  sessionId: string;
  closedAt: string;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const queryClient = useQueryClient();
  return (
    <div
      role="status"
      aria-label="Session closed"
      className="flex flex-wrap items-center gap-3 bg-surface-container-low px-3 py-2"
    >
      <span className="font-code-sm text-code-sm text-on-surface-variant">
        Session closed {formatTimestamp(closedAt)} — history is read-only.
        Reopen to bring its participants back and continue.
      </span>
      <Button
        type="button"
        variant="primary"
        size="sm"
        disabled={busy}
        onClick={async () => {
          setBusy(true);
          setError(null);
          try {
            await invoke("reopen_session", { sessionId });
            // The row moved: re-read it so the live composer replaces this
            // bar, and the dashboard lists the session again.
            await queryClient.invalidateQueries({
              queryKey: ["get_session", { sessionId }],
            });
            await queryClient.invalidateQueries({ queryKey: ["list_sessions"] });
          } catch (e) {
            setError(errorMessage(e));
          } finally {
            setBusy(false);
          }
        }}
      >
        {busy ? "Reopening…" : "Reopen"}
      </Button>
      {error && (
        <ErrorBanner
          label="Reopen failed:"
          message={error}
          onDismiss={() => setError(null)}
          className="basis-full"
        />
      )}
    </div>
  );
}
