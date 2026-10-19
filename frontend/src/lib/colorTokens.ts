/**
 * The detector behind `colorTokens.test.ts`'s sweep (colours come from the
 * theme, the 2026-10-19 visual pass, slice S1e) — split out so the codebase map
 * has a subject to place, the same shape as `typeScale.ts`. The sweep skips
 * this one file: a detector has to spell what it detects.
 */
import { stripComments } from "./framing";

export type ColorRule = "raw-hex";

/**
 * A hex colour (`#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`) where a colour VALUE
 * stands: right after a quote, a backtick, `[` (a Tailwind arbitrary value,
 * `bg-[#123456]`), `(` or `,` (a gradient's stops). A reference in running
 * text ("feedback #66a", "fixed in #103") is not one.
 */
export const COLOR_RULES: Record<ColorRule, { pattern: RegExp; fix: string }> = {
  "raw-hex": {
    pattern: /(?<=["'`[(,]\s*)#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6}|[0-9a-fA-F]{3,4})(?![\w-])/,
    fix: "a raw hex colour — use a theme token from tailwind.config.ts (a class, or the config object where a library takes a literal)",
  },
};

export interface ColorHit {
  line: number;
  rule: ColorRule;
  text: string;
}

/** Every raw colour on a non-comment line of `source`, by 1-based line. */
export function findRawColors(source: string): ColorHit[] {
  const hits: ColorHit[] = [];
  stripComments(source)
    .split("\n")
    .forEach((text, i) => {
      for (const rule of Object.keys(COLOR_RULES) as ColorRule[]) {
        if (COLOR_RULES[rule].pattern.test(text)) {
          hits.push({ line: i + 1, rule, text: text.trim() });
        }
      }
    });
  return hits;
}
