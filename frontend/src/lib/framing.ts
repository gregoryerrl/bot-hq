// Detection for retired agent-framing in user-facing strings. Pure — no `node:`
// imports — so it stays safe to sit in `src/lib/`; the file walking lives in
// `framing.test.ts`, which is the only caller.
//
// House rule: bot-hq is an agent harness, never framed by agent count. A session
// runs N participants (dialog default 1, cap 4) and roles are the user's own
// configuration, so naming a pair describes a session shape most rosters do not
// have. The retired names Brian and Rain survive only in pre-rc3 history and in
// the external driver's wire fields, both deliberately.

/**
 * Word-boundary matched with `_` counted as a word character — the same rule as
 * the Rust `contains_word`, and load-bearing here: `session.rain_enabled` is a
 * live field name (a DB column named for a retired agent, round-2 finding E2),
 * and a substring check would flag it on every render path reading that flag.
 */
export const RETIRED_FRAMING =
  /(?<![A-Za-z0-9_])(brian|rain|duo)(?![A-Za-z0-9_])/gi;

/**
 * Strip `//` line and block comments.
 *
 * Comments are exempt on purpose: a comment describing a 2026-07-10 incident in
 * that day's vocabulary is a RECORD, not drift — the same call `146736d` made
 * when it fixed `GENERAL_RULES` and deliberately left the comment layer alone.
 * What ships to a user's screen is what gets guarded.
 *
 * The `[^:]` guard before `//` keeps `https://` out of the line-comment rule.
 */
export function stripComments(source: string): string {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .split("\n")
    .map((l) => l.replace(/(^|[^:])\/\/.*$/, "$1"))
    .join("\n");
}

/** One offending line: its 1-based number, the words matched, and the text. */
export interface FramingHit {
  line: number;
  words: string[];
  text: string;
}

/** Every non-comment line of `source` that names a retired agent or a pair. */
export function findRetiredFraming(source: string): FramingHit[] {
  const hits: FramingHit[] = [];
  stripComments(source)
    .split("\n")
    .forEach((text, i) => {
      const words = text.match(RETIRED_FRAMING);
      if (words) hits.push({ line: i + 1, words, text: text.trim() });
    });
  return hits;
}
