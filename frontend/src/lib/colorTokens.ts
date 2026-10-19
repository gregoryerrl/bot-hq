/**
 * The detector behind `colorTokens.test.ts`'s sweep (colours come from the
 * theme, the 2026-10-19 visual pass, slice S1e) — split out so the codebase map
 * has a subject to place, the same shape as `typeScale.ts`. The sweep skips
 * this one file: a detector has to spell what it detects.
 */
import { stripComments } from "./framing";

export type ColorRule = "raw-hex" | "raw-palette";

const UTILITIES = "text|bg|border|ring|outline|fill|stroke|from|via|to|divide|placeholder|decoration|shadow|accent|caret";
const HUES = "slate|gray|zinc|neutral|stone|red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose";

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
  // A Tailwind DEFAULT palette class (`bg-black/60`, `text-gray-400`), not a
  // theme token: the default palette sits outside the design's colours, so a
  // stock grey or black cannot be retuned with the theme (the dialog scrims
  // were bg-black until S2f made them the `scrim` token). The participant
  // palette's names (`text-author-orange`) are tokens and never match: the
  // colour must follow the utility directly.
  "raw-palette": {
    pattern: new RegExp(
      `(?<![\\w-])(?:${UTILITIES})-(?:(?:${HUES})-(?:50|[1-9]00|950)|black|white)(?![\\w-])`,
    ),
    fix: "a Tailwind default palette colour — use a theme token (bg-scrim, text-on-surface, text-muted, …)",
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
