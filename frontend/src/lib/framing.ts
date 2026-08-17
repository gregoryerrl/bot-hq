// Detection for retired agent-framing in user-facing strings. Pure — no `node:`
// imports — so it stays safe to sit in `src/lib/`; the file walking lives in
// `framing.test.ts`, which is the only caller.
//
// House rule: bot-hq is an agent harness, never framed by agent count. A session
// runs N participants (dialog default 1, cap 4) and roles are the user's own
// configuration, so naming a pair describes a session shape most rosters do not
// have. The retired names Brian and Rain survive only in pre-rc3 history (and
// the frozen `paths.rs` legacy-seed constants), deliberately.

/**
 * Word-boundary matched with `_` counted as a word character — the same rule as
 * the Rust `contains_word`, and still load-bearing after the hard retirement:
 * `rain_disabled_default` is a live identifier (the settings key D13 deleted,
 * kept as a legacy fixture in `Settings.test.tsx`), and a substring check would
 * flag it. It used to be justified by `session.rain_enabled`, a DB column named
 * for a retired agent — that column is gone as of migration 0060, so the
 * example moved but the rule did not.
 */
const RETIRED_FRAMING =
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
 *
 * **Block comments are blanked, not deleted** (round 5, N5). They used to be
 * replaced with `""` — newlines and all — and every caller here splits the
 * STRIPPED text and reports `i + 1` as the line number, so each multi-line block
 * comment shifted every subsequent report upward by its own height. Measured: an
 * offender on line 5, preceded by a three-line block, was reported as line 3.
 * Detection was never affected — the guard failed the build either way — but the
 * number it printed sent the reader to the wrong line, and the drift grows with
 * every doc comment above the hit. Replacing each non-newline character with a
 * space keeps line count AND column offsets intact.
 */
export function stripComments(source: string): string {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, (block) => block.replace(/[^\n]/g, " "))
    .split("\n")
    .map((l) => l.replace(/(^|[^:])\/\/.*$/, "$1"))
    .join("\n");
}

/** One offending line: its 1-based number, the words matched, and the text. */
interface FramingHit {
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
