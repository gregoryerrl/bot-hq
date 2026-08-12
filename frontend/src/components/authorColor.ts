/**
 * Author SLUG → colour class. Keys are internal wire values (the `author`
 * column, the busy map), never rendered — rc3 D10 puts `ROLE · Model` on screen
 * and this only tints it. An unmapped slug falls back to the neutral tone.
 */
const colorByAuthor: Record<string, string> = {
  brian: "text-author-brian",
  rain: "text-author-rain",
  user: "text-author-user",
  system: "text-on-surface-variant",
};

export function authorColorClass(author: string) {
  return colorByAuthor[author] ?? "text-on-surface-variant";
}
