// @vitest-environment node
/**
 * **One type scale** (the 2026-10-19 visual pass, slice S1c). DESIGN.md's five
 * type tokens — `headline-lg`, `headline-md`, `body-md`, `code-sm`,
 * `label-caps` — are Tailwind `fontSize` + `fontFamily` keys in
 * `tailwind.config.ts`, used as pairs (`font-code-sm text-code-sm`). Beside them
 * the tree had grown 30 arbitrary sizes (`text-[0.7rem]`, `text-[0.65rem]`,
 * `text-[10px]`, …), six uses of `font-body` — a class Tailwind never generates,
 * so that text silently took its parent's family — and 26 of `font-mono`, the
 * same JetBrains Mono as `font-code-sm` under a second name. All three are gone;
 * this sweep keeps them gone.
 *
 * Swept: every `.ts`/`.tsx` under `src/` except test files, which are never
 * rendered (a fixture may spell an old class on purpose) — and which is why the
 * patterns live HERE, in the test, with no detector module needing an
 * exemption. Comments are records, not classes: they are stripped first, with
 * the `stripComments` that `framing.test.ts` and `overflow.test.ts` share.
 *
 * `import.meta.glob` rather than `node:fs`, for `overflow.test.ts`'s stated
 * reason: the tsconfig carries no `@types/node`, so `readdirSync`/`__dirname`
 * fail `tsc --noEmit`. Nothing here needs a DOM, hence the node environment.
 *
 * Since S5 it also refuses Tailwind's DEFAULT sizes (`text-xs`, `text-sm`,
 * `text-base`, …): their last uses moved onto the tokens.
 *
 * Not seen by this sweep (measured 2026-10-19, so "0 violations" is not read as
 * "0 possible"): `.css` files; inline
 * `style={{ fontSize }}`; a class assembled by string concatenation; xterm's own
 * `fontSize` option (`SessionTerminalTab.tsx`, a renderer setting, not a class).
 */
import { describe, expect, it } from "vitest";
import { findTypeScaleViolations, RULES, type Rule } from "./typeScale";

const SOURCES = import.meta.glob("../**/*.{ts,tsx}", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

const isTest = (p: string) => /\.test\.tsx?$/.test(p);

/**
 * Sites that must keep a banned class: the file (path under `src/`), the rule,
 * and why. EMPTY, and meant to stay that way — an entry is a decision written
 * down where it can be audited, and the last test below fails the moment an
 * entry stops matching anything.
 */
const ALLOWLIST: { file: string; rule: Rule; reason: string }[] = [];

// The detector itself spells its rules (the rule names, the fix text), so it
// is the one file the sweep skips — never a rendered component.
// (Vite keys a same-folder file as "./typeScale.ts".)
const isDetector = (p: string) => p === "./typeScale.ts" || p.endsWith("/lib/typeScale.ts");
const swept = () => Object.keys(SOURCES).filter((p) => !isTest(p) && !isDetector(p));
const allowed = (file: string, rule: Rule) =>
  ALLOWLIST.some((a) => a.rule === rule && file.endsWith(`/${a.file}`));

describe("one type scale", () => {
  it("no source file uses an arbitrary or default text size, font-body or font-mono", () => {
    const offenders = swept().flatMap((file) =>
      findTypeScaleViolations(SOURCES[file])
        .filter((h) => !allowed(file, h.rule))
        .map((h) => `${file.replace("../", "")}:${h.line}: ${RULES[h.rule].fix} — ${h.text}`),
    );
    expect(offenders).toEqual([]);
  });

  /** The sweep must actually be reading files, not an empty directory. */
  it("reads the frontend source tree", () => {
    expect(swept().length).toBeGreaterThan(60);
  });

  /**
   * A guard that passes on first run proves nothing. This pins the
   * DISCRIMINATION, so a later edit to a pattern can neither make the sweep
   * vacuous nor start flagging the tokens themselves.
   */
  it("flags each banned form and passes the tokens", () => {
    const rulesOf = (src: string) => findTypeScaleViolations(src).map((h) => h.rule);

    expect(rulesOf(`<p className="mt-1 text-[0.7rem] text-muted">`)).toEqual(["arbitrary-text"]);
    expect(rulesOf(`cn("px-1", "md:text-[10px]")`)).toEqual(["arbitrary-text"]);
    expect(rulesOf(`<span className="font-body text-code-sm">`)).toEqual(["font-body"]);
    expect(rulesOf(`<span className="block font-body">`)).toEqual(["font-body"]);
    expect(rulesOf(`cn("shrink-0 font-mono", running && "text-primary")`)).toEqual(["font-mono"]);
    expect(rulesOf("`font-mono ${x}`")).toEqual(["font-mono"]);
    expect(rulesOf(`<p className="mt-1 text-xs text-muted">`)).toEqual(["tailwind-size"]);
    expect(rulesOf(`cn("px-2", "md:text-sm")`)).toEqual(["tailwind-size"]);
    expect(rulesOf(`<h1 className="text-2xl font-semibold">`)).toEqual(["tailwind-size"]);

    for (const ok of [
      `<h2 className="font-headline-lg text-headline-lg">`,
      `<h3 className="font-headline-md text-headline-md">`,
      `<p className="font-body-md text-body-md text-on-surface-variant">`,
      `<code className="font-code-sm text-code-sm">`,
      `<span className="font-label-caps text-label-caps">`,
      // Colours and other arbitrary values are not this guard's business.
      `<div className="min-w-[1.25rem] max-w-[12rem] text-on-surface [overflow-wrap:anywhere]">`,
    ]) {
      expect(rulesOf(ok), ok).toEqual([]);
    }
  });

  it("does not read comments as classes, and reports the real line number", () => {
    const src = [
      `// font-mono was retired for font-code-sm`,
      `{/* was text-[0.7rem]`,
      `    and font-body */}`,
      `<span className="font-mono" />`,
    ].join("\n");
    expect(findTypeScaleViolations(src)).toEqual([
      { line: 4, rule: "font-mono", text: `<span className="font-mono" />` },
    ]);
  });

  /**
   * An allowlist entry must still be needed: a stale one is the hole the next
   * violation walks in through (the rule `overflow.test.ts` keeps for its own
   * exemption).
   */
  it("keeps every allowlist entry earning its place", () => {
    for (const entry of ALLOWLIST) {
      const key = Object.keys(SOURCES).find((k) => k.endsWith(`/${entry.file}`));
      expect(key, `${entry.file} must be in the swept set`).toBeDefined();
      expect(entry.reason.trim().length, `${entry.file}: say why`).toBeGreaterThan(20);
      expect(
        findTypeScaleViolations(SOURCES[key as string]).some((h) => h.rule === entry.rule),
        `${entry.file} no longer needs its ${entry.rule} entry — delete it`,
      ).toBe(true);
    }
  });
});
