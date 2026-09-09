/** One participant's delivery lag (WS1c, 2026-08-27) — the visibility half of
 *  the anti-starvation work. Hand-defined mirror of the Rust
 *  `ParticipantBacklogView` (bindings.ts regenerates only at app launch). */
export type ParticipantBacklog = {
  participant_id: number;
  slug: string;
  last_delivered_at: string | null;
  undelivered_peer_texts: number;
  /** Computed by the BACKEND from the scheduler's own summons threshold
   *  (`STARVATION_SUMMONS_MIN_PEER_TEXTS`) — the chip carries no threshold of
   *  its own, so what the user sees and what the ring acts on are one Rust
   *  constant, not two literals that can drift (EYES A6). */
  starving: boolean;
};

/** Minutes since `last_delivered_at`, or undefined when never dealt / unparsable. */
export function minutesSince(iso: string | null, now: number): number | undefined {
  if (!iso) return undefined;
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return undefined;
  return Math.max(0, Math.round((now - t) / 60_000));
}

/** Renders nothing unless the BACKEND says starving — a healthy roster shows
 *  no chip at all. Past it: an amber pill with the unread count, and the full
 *  story in the tooltip. 137 starvation events across 38 sessions went
 *  undiagnosed because a starved participant was indistinguishable from a
 *  quiet one in this UI. */
export function BacklogChip({
  backlog,
  name,
  now = Date.now(),
}: {
  backlog?: ParticipantBacklog;
  name: string;
  /** Injectable for tests. */
  now?: number;
}) {
  if (!backlog?.starving) {
    return null;
  }
  const mins = minutesSince(backlog.last_delivered_at, now);
  const dealt =
    mins === undefined ? "never been dealt a turn" : `last dealt a turn ${mins}m ago`;
  return (
    <span
      className="ml-1 rounded border border-warning/50 bg-warning/15 px-1 py-px align-middle font-label-caps text-label-caps text-warning"
      title={
        `${name} has ${backlog.undelivered_peer_texts} peer messages undelivered and has ` +
        `${dealt}. The ring serves it one turn on the next user message ` +
        `(anti-starvation summons); typing @${backlog.slug} summons it now.`
      }
    >
      {backlog.undelivered_peer_texts} unread
    </span>
  );
}
