/**
 * The detector behind `typeScale.test.ts`'s sweep (one type scale, the
 * 2026-10-19 visual pass, slice S1c) — split out so the codebase map has a
 * subject to place, the same shape as `overflow.ts`. The sweep skips this one
 * file: a detector has to spell what it detects.
 */
import { stripComments } from "./framing";

const BODY = "font-body";
const MONO = "font-mono";

export type Rule = "arbitrary-text" | "font-body" | "font-mono";

/**
 * Each banned class, matched as a whole class token: not preceded by a word
 * character or `-` — so a variant prefix (`md:`, `hover:`) or `!` still matches
 * and `subtext-[` does not — and, for the two family names, not followed by one
 * either, so `font-body-md` is fine.
 */
export const RULES: Record<Rule, { pattern: RegExp; fix: string }> = {
  "arbitrary-text": {
    pattern: /(?<![\w-])text-\[/,
    fix: "arbitrary text-[…] value — use a type token (text-headline-lg / text-headline-md / text-body-md / text-code-sm / text-label-caps)",
  },
  "font-body": {
    pattern: new RegExp(`(?<![\\w-])${BODY}(?![\\w-])`),
    fix: `${BODY} is not a class — use font-body-md (prose) or font-code-sm (mono)`,
  },
  "font-mono": {
    pattern: new RegExp(`(?<![\\w-])${MONO}(?![\\w-])`),
    fix: `${MONO} duplicates font-code-sm (the same JetBrains Mono) — use font-code-sm`,
  },
};

export interface Hit {
  line: number;
  rule: Rule;
  text: string;
}

/** Every banned class on a non-comment line of `source`, by 1-based line. */
export function findTypeScaleViolations(source: string): Hit[] {
  const hits: Hit[] = [];
  stripComments(source)
    .split("\n")
    .forEach((text, i) => {
      for (const rule of Object.keys(RULES) as Rule[]) {
        if (RULES[rule].pattern.test(text)) {
          hits.push({ line: i + 1, rule, text: text.trim() });
        }
      }
    });
  return hits;
}
