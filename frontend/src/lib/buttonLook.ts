/**
 * The detector behind `buttonLook.test.ts`'s sweep (one button, the
 * 2026-10-19 visual pass, S2g): a raw `<button>` that draws its OWN button
 * look — a boxed outline (`rounded` + `border` + padding) or the primary fill
 * — instead of being the kit's `<Button>`. S2b moved 72 of them onto the kit;
 * this keeps new ones from growing back. A raw button that is a row, a tab, a
 * switch or a toggle draws no such box and is not flagged.
 */
import { stripComments } from "./framing";

export interface ButtonLookHit {
  line: number;
  look: "boxed" | "filled";
  classes: string;
}

/** The text of a JSX opening tag from `start` (just past `<button`) to its
 *  closing `>` — skipping any `>` inside a quote or an expression (`=>`). */
function tagBody(src: string, start: number): string {
  let depth = 0;
  let quote: string | null = null;
  for (let i = start; i < src.length; i++) {
    const ch = src[i];
    if (quote) {
      if (ch === quote && src[i - 1] !== "\\") quote = null;
    } else if (ch === '"' || ch === "'" || ch === "`") quote = ch;
    else if (ch === "{") depth++;
    else if (ch === "}") depth--;
    else if (ch === ">" && depth === 0) return src.slice(start, i);
  }
  return src.slice(start);
}

/** Every raw `<button>` in `source` that draws a button look, by 1-based line
 *  of the `<button`. Classes are read from every string literal in the tag,
 *  so a `cn(…)` with branches counts each branch's classes. */
export function findHandRolledButtons(raw: string): ButtonLookHit[] {
  // Comments first (the other sweeps' `stripComments`, which keeps the line
  // numbers): an apostrophe in a `{/* … */}` inside a tag would otherwise
  // open a "quote" and run the tag on into the rest of the file.
  const source = stripComments(raw);
  const hits: ButtonLookHit[] = [];
  const open = /<button\b/g;
  for (let m = open.exec(source); m; m = open.exec(source)) {
    const body = tagBody(source, m.index + m[0].length);
    const literals = [...body.matchAll(/"([^"]*)"|`([^`]*)`/g)].map((x) => x[1] ?? x[2] ?? "");
    const classes = literals.join(" ").split(/\s+/).filter(Boolean);
    const has = (c: string) => classes.includes(c);
    const sideBorder = classes.some((c) => /^border-[btlrxy](-|$)/.test(c));
    const boxed = has("rounded") && has("border") && !sideBorder && classes.some((c) => /^p[xy]?-/.test(c));
    const filled = has("bg-primary");
    if (boxed || filled) {
      hits.push({
        line: source.slice(0, m.index).split("\n").length,
        look: filled ? "filled" : "boxed",
        classes: classes.join(" "),
      });
    }
  }
  return hits;
}
