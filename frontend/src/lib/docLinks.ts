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
