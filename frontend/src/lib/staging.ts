/**
 * The staged-answer snapshot the backend holds (`get_staged_response`) vs the
 * tray picks the user has staged since — the pure half of SessionView's
 * re-stage effect (round 12).
 *
 * The effect used to be keyed on the pick COUNT, so changing an already-staged
 * answer's VALUE (click option A, then B on the same tray row) never re-ran it:
 * the backend kept A and delivered A at the next boundary. `stagedKey` is the
 * dependency now — it moves on every value change and on every add/remove —
 * and `picksDiffer` is the comparison it runs.
 */
export type StagedPick = { choice_id: string; picked: string };

/** A stable key over the staged map: moves when a pick is added, removed, OR
 *  changed. `Object.keys(map).length` (the old key) misses the third. */
export function stagedKey(map: Record<string, string>): string {
  return JSON.stringify(
    Object.entries(map).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)),
  );
}

/** Whether the picks currently staged differ from the backend's snapshot —
 *  by count, by choice, or by value, in order. */
export function picksDiffer(
  current: readonly StagedPick[],
  staged: readonly StagedPick[],
): boolean {
  if (current.length !== staged.length) return true;
  return current.some(
    (p, i) => staged[i]?.choice_id !== p.choice_id || staged[i]?.picked !== p.picked,
  );
}
