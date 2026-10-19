// @vitest-environment node
/**
 * **Colours come from the theme** (the 2026-10-19 visual pass, slice S1e). Every
 * colour in the app is a token in `tailwind.config.ts` — DESIGN.md's palette,
 * the participant palette and the S1b greys — so no source file spells a hex
 * value. Before S1e the one place that did was the Terminal tab's xterm theme,
 * which restated four tokens as hex; it now reads them from the config
 * (`xtermTheme.ts`), and the last test below pins that.
 *
 * Swept: every `.ts`/`.tsx` under `src/` except test files (a fixture may
 * spell a colour on purpose) and the detector itself. Comments are stripped
 * first, with the `stripComments` the other sweeps share; `import.meta.glob`
 * for `typeScale.test.ts`'s reason (no `@types/node` in the tsconfig).
 *
 * Since S2f it also refuses a Tailwind DEFAULT palette class (`bg-black/60`,
 * `text-gray-400`): the dialog scrims were the last of them, and they are the
 * theme's `scrim` token now.
 *
 * Not seen by this sweep (so "0 violations" is not read as "0 possible"):
 * `.css` files (`index.css` uses `theme()`); `rgb()`/`hsl()` literals; a colour
 * assembled by string concatenation.
 */
import { describe, expect, it } from "vitest";
import tailwindConfig from "../../tailwind.config";
import { COLOR_RULES, findRawColors, type ColorRule } from "./colorTokens";
import { XTERM_THEME } from "./xtermTheme";

const SOURCES = import.meta.glob("../**/*.{ts,tsx}", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

const isTest = (p: string) => /\.test\.tsx?$/.test(p);

/**
 * Sites that must keep a raw colour: the file (path under `src/`), the rule,
 * and why. EMPTY — an entry is a decision written down where it can be
 * audited, and the allowlist test fails the moment an entry stops matching.
 */
const ALLOWLIST: { file: string; rule: ColorRule; reason: string }[] = [];

// Vite keys a same-folder file as "./colorTokens.ts".
const isDetector = (p: string) => p === "./colorTokens.ts" || p.endsWith("/lib/colorTokens.ts");
const swept = () => Object.keys(SOURCES).filter((p) => !isTest(p) && !isDetector(p));
const allowed = (file: string, rule: ColorRule) =>
  ALLOWLIST.some((a) => a.rule === rule && file.endsWith(`/${a.file}`));

describe("colours come from the theme", () => {
  it("no source file spells a raw hex colour or a default palette class", () => {
    const offenders = swept().flatMap((file) =>
      findRawColors(SOURCES[file])
        .filter((h) => !allowed(file, h.rule))
        .map((h) => `${file.replace("../", "")}:${h.line}: ${COLOR_RULES[h.rule].fix} — ${h.text}`),
    );
    expect(offenders).toEqual([]);
  });

  it("reads the frontend source tree", () => {
    expect(swept().length).toBeGreaterThan(60);
  });

  /** Pins the DISCRIMINATION, so a pattern edit can neither make the sweep
   *  vacuous nor start flagging issue references in prose. */
  it("flags a colour value and passes a reference in running text", () => {
    const rulesOf = (src: string) => findRawColors(src).map((h) => h.rule);

    for (const bad of [
      `background: "#0b1326",`,
      `style={{ color: '#fff' }}`,
      "const c = `#ffb68bcc`;",
      `<div className="bg-[#123456] text-on-surface">`,
      `"linear-gradient(#000, #abcdef)"`,
    ]) {
      expect(rulesOf(bad), bad).toEqual(["raw-hex"]);
    }
    for (const ok of [
      `{/* Feedback #66a: approved commands */}`,
      `<p>Fixed by feedback #103 and #66a.</p>`,
      `"see issue #104 for the chip"`,
      `<span className="bg-surface text-on-surface border-outline-variant" />`,
      `const ref = "abc#def";`,
      `"&#39;"`,
    ]) {
      expect(rulesOf(ok), ok).toEqual([]);
    }
  });

  it("flags a default palette class and passes the theme's tokens", () => {
    const rulesOf = (src: string) => findRawColors(src).map((h) => h.rule);
    for (const bad of [
      `<div className="fixed inset-0 z-40 bg-black/60" />`,
      `cn("text-gray-400", x)`,
      `"hover:bg-red-500"`,
      `<p className="border-slate-800 text-white">`,
    ]) {
      expect(rulesOf(bad), bad).toEqual(["raw-palette"]);
    }
    for (const ok of [
      `<div className="fixed inset-0 z-40 bg-scrim/60" />`,
      `<span className="text-author-orange bg-author-cyan/15" />`,
      `<p className="text-on-surface text-muted border-outline-variant bg-surface-container-lowest" />`,
      `<span className="text-warning bg-warning/15 border-primary/40" />`,
      `"a white paper, a black box"`,
    ]) {
      expect(rulesOf(ok), ok).toEqual([]);
    }
  });

  it("does not read comments as colours, and reports the real line number", () => {
    const src = [`// was #0b1326`, `/* and #fff`, `   */`, `const x = "#dae2fd";`].join("\n");
    expect(findRawColors(src)).toEqual([{ line: 4, rule: "raw-hex", text: `const x = "#dae2fd";` }]);
  });

  it("keeps every allowlist entry earning its place", () => {
    for (const entry of ALLOWLIST) {
      const key = Object.keys(SOURCES).find((k) => k.endsWith(`/${entry.file}`));
      expect(key, `${entry.file} must be in the swept set`).toBeDefined();
      expect(entry.reason.trim().length, `${entry.file}: say why`).toBeGreaterThan(20);
      expect(
        findRawColors(SOURCES[key as string]).some((h) => h.rule === entry.rule),
        `${entry.file} no longer needs its ${entry.rule} entry — delete it`,
      ).toBe(true);
    }
  });

  /** The terminal is drawn with the app's own surface and text colours. */
  it("gives the terminal the theme's colours", () => {
    const c = tailwindConfig.theme.extend.colors;
    expect(XTERM_THEME).toEqual({
      background: c.surface,
      foreground: c["on-surface"],
      cursor: c["on-surface"],
      selectionBackground: c["surface-container-highest"],
    });
    for (const v of Object.values(XTERM_THEME)) expect(v).toMatch(/^#[0-9a-f]{6}$/i);
  });
});
