import { describe, expect, it } from "vitest";
import { findRetiredFraming, stripComments } from "./framing";

/**
 * **Nothing the USER reads names a retired agent, or assumes there are two.**
 *
 * The Rust side has had this guard since round 1
 * (`no_tool_description_an_agent_reads_names_an_agent`), scoped to the prose
 * AGENTS read. Nothing covered the prose USERS read, and round 2 found three
 * survivors of round 1's framing sweep for exactly that reason — they were
 * rendered strings rather than doc prose, so a sweep over `GENERAL_RULES`, the
 * public site, README and ARCHITECTURE went straight past them:
 *
 *   - `Settings.tsx` — "the duo re-spawns via --resume" in Archived Sessions
 *   - `SessionView.tsx` + `SessionTile.tsx` — the NEEDS DIRECTION tooltip,
 *     duplicated verbatim in both, so fixing one left the other stale
 */
describe("framing detection", () => {
  it("ignores comments and identifiers, catches rendered text", () => {
    // The three shapes that must NOT trip it, all real lines from this tree.
    expect(findRetiredFraming("// the duo was nudged to declare state")).toEqual(
      [],
    );
    expect(findRetiredFraming("/* Brian and Rain are legacy */")).toEqual([]);
    expect(findRetiredFraming("const k = \"rain_disabled_default\";")).toEqual(
      [],
    );
    // And the shape that must.
    const hit = findRetiredFraming('  title="the duo was nudged"');
    expect(hit).toHaveLength(1);
    expect(hit[0].words).toEqual(["duo"]);
  });

  it("keeps a url out of the line-comment rule", () => {
    // `https://` must not read as a comment start, or everything after a link
    // on the same line stops being scanned.
    expect(stripComments('const u = "https://x/duo";')).toContain("duo");
  });

  /**
   * **A stripped block comment must not move the lines under it** — round 5's
   * N5. Block comments were replaced with `""`, newlines included, and every
   * caller splits the STRIPPED text and reports `i + 1`, so each multi-line
   * block shifted every later report upward by its own height. Detection was
   * never wrong; the line number printed beside it was, and it pointed the
   * reader at innocent code.
   *
   * Measured before the fix: an offender on line 5, under a three-line block,
   * was reported as line 3. Blanking to spaces preserves both the line count and
   * the column offsets.
   */
  it("does not shift line numbers when a block comment is stripped", () => {
    const src = [
      "const a = 1;",
      "/* a block",
      "   spanning",
      "   three lines */",
      'const t = "the duo was nudged";',
    ].join("\n");

    expect(stripComments(src).split("\n")).toHaveLength(5);

    const hits = findRetiredFraming(src);
    expect(hits).toHaveLength(1);
    expect(hits[0].line).toBe(5);
  });
});

describe("user-facing framing", () => {
  /**
   * Every source file under `src/`, as raw text.
   *
   * `import.meta.glob` rather than `node:fs`: this project's tsconfig carries
   * no `@types/node`, so `readdirSync`/`process` fail `tsc --noEmit` — a gate,
   * not a preference. Vite resolves the glob at transform time, which also
   * means a file added to the tree is picked up with no list to maintain.
   */
  const SOURCES = import.meta.glob("../**/*.{ts,tsx}", {
    query: "?raw",
    import: "default",
    eager: true,
  }) as Record<string, string>;

  /**
   * THREE exemptions, each for a reason that is a property of the file rather
   * than a judgement about its contents — which is what keeps this from being
   * the sanctioned-absence list that rots into permission for the next one.
   * All three are named here; an unstated exemption is the one that rots.
   *
   * - `*.test.ts(x)` — a test file is never rendered to a screen. A component
   *   whose string regresses is still caught in the COMPONENT, even when its
   *   own test asserts the new text, so sweeping tests would only flag the
   *   fixtures that quote the old wording on purpose. (This one was a silent
   *   filter in the predicate below until the reviewer named it.)
   * - `bindings.ts` — GENERATED from Rust at app launch and `@ts-nocheck`. Its
   *   content is the wire, which keeps the retired names deliberately (the
   *   external driver's fields), and no human can author a regression into it:
   *   an edit here is overwritten on next launch. The exemption cannot become
   *   a hiding place.
   * - `framing.ts` — DEFINES the pattern, so it necessarily spells the words it
   *   searches for. Building the regex from fragments to slip past its own
   *   check would be rewording around the gate — the exact move the general
   *   rules forbid — so the exemption is stated where it is auditable instead
   *   of hidden in a clever regex.
   */
  const EXEMPT_FILES = ["bindings.ts", "framing.ts"];
  const isExempt = (p: string) =>
    /\.test\.tsx?$/.test(p) || EXEMPT_FILES.some((e) => p.endsWith(`/${e}`));

  it("no rendered string names a retired agent or assumes a pair", () => {
    const files = Object.keys(SOURCES).filter((p) => !isExempt(p));
    // The sweep is only meaningful if it actually walked the tree; an empty
    // file list would pass silently and pin nothing.
    expect(files.length).toBeGreaterThan(40);

    const offenders = files.flatMap((file) =>
      findRetiredFraming(SOURCES[file]).map(
        (h) =>
          `${file.replace("../", "")}:${h.line}: ${h.words.join(", ")} — ${h.text}`,
      ),
    );
    expect(offenders).toEqual([]);
  });
});
