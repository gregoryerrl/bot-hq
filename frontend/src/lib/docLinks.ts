/** The session-document slugs a tray question names in backticks (feedback
 *  #79): an agent that parks "Approve the key questions?" and writes the doc's
 *  name inside the prose — `key-questions` — gets a link from the tray card
 *  to that doc's tab, with no new tool parameter. Only a backticked token
 *  that IS an existing slug counts (a code span naming a file or a command
 *  stays prose); each slug once, in the order it appears. */
export function docSlugsIn(prompt: string, slugs: readonly string[]): string[] {
  const known = new Set(slugs);
  const out: string[] = [];
  for (const m of prompt.matchAll(/`([^`\n]{1,80})`/g)) {
    const token = m[1]?.trim() ?? "";
    if (known.has(token) && !out.includes(token)) out.push(token);
  }
  return out;
}

/** Where a doc link opens (EYES 13f5caad): a doc with a phase tag — the
 *  phase doc itself (`plan`) or the reviewer's notes (`plan-eyes`) — lives in
 *  that phase's tab, and ONLY an untagged doc is a custom tab. Sending a
 *  tagged slug to the custom tab opened a blank pane. `null` for a slug that
 *  is no live doc. */
export function docTabTarget(
  slug: string,
  docs: readonly { slug: string; phase: string | null }[],
): { phase: string } | { custom: string } | null {
  const doc = docs.find((d) => d.slug === slug);
  if (!doc) return null;
  return doc.phase ? { phase: doc.phase } : { custom: slug };
}
