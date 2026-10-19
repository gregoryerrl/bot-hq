/**
 * The short form of a session id for compact UI (`s-69ef196a` → `s-69ef19`):
 * the first 8 characters, which is the `s-` prefix plus six hex — enough to
 * tell sessions apart in a list, short enough for a chip. One helper (round 9:
 * four call sites spelled `id.slice(0, 8)` independently). Since the visual
 * pass the dashboard tile uses it too: its `S-XXXX` was a third form.
 */
export function shortSessionId(id: string): string {
  return id.slice(0, 8);
}
