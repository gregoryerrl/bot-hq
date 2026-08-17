/**
 * **No horizontal scrolling, ever** — the user's mandate, recorded in the CL as
 * absolute, and this is the first mechanical enforcement of it.
 *
 * `ApprovalGate.test.tsx` has asserted the pair since a long command ran off the
 * right edge of a gate card. It asserts it for ONE BOX, read by role. Round 5
 * found NINE bare containers — including `ApprovalGate.tsx:260`, a sibling inside
 * that same component, which is why the detection is line-scoped and not
 * file-scoped. One of the nine is the @mention picker, whose rows render a
 * user-typed participant label (rc3 D20), so its width is user-controlled.
 *
 * A rule with one guarded box and nine unguarded containers is not enforced.
 * This sweeps the tree; `overflow.ts` holds the pure detection, exactly as
 * `framing.ts` does for retired framing.
 *
 * **No CONTAINER is exempt.** `DocumentPane`'s pre element cannot overflow
 * (`whitespace-pre-wrap` + `[overflow-wrap:anywhere]`), so it is the obvious
 * carve-out — and it is paired anyway. The retired-identifier guard's exemption
 * list only stayed honest because it was forced to shrink; a guard that ships
 * with an exemption ships with somewhere for the next violation to live.
 *
 * `overflow.ts` itself is excluded, and that is a different thing from a
 * container carve-out: it DEFINES the class list, so it necessarily spells the
 * strings it matches. `framing.test.ts` carries the same exclusion for
 * `framing.ts`, with the same reason, and this follows it rather than inventing
 * a second convention.
 */
import { describe, expect, it } from "vitest";
import {
  HORIZONTAL_CLIP,
  SCROLL_AXIS_CLASSES,
  findBareScrollContainers,
} from "./overflow";

/**
 * Every source file under `src/`, as raw text.
 *
 * `import.meta.glob` rather than `node:fs`, and this is `framing.test.ts`'s
 * stated reason rather than a style choice: the project's tsconfig carries no
 * `@types/node`, so `readdirSync`/`__dirname` fail `tsc --noEmit` — gate 3.
 * Written the other way first and caught by that gate. Vite resolves the glob at
 * transform time, so a file added to the tree is swept with no list to maintain.
 */
const SOURCES = import.meta.glob("../**/*.{ts,tsx}", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

/**
 * The detector DEFINES the class list, so it necessarily spells every string it
 * matches. Same exclusion, same reason, as `framing.test.ts`'s `EXEMPT_FILES`.
 * Asserted below to still be earning its place, so a stale carve-out cannot
 * quietly become the hole the next bare container enters through.
 */
const EXEMPT_FILES = ["overflow.ts"];
const isExempt = (p: string) =>
  /\.test\.tsx?$/.test(p) || EXEMPT_FILES.some((e) => p.endsWith(`/${e}`));

function offenders(): string[] {
  return Object.keys(SOURCES)
    .filter((p) => !isExempt(p))
    .flatMap((file) =>
      findBareScrollContainers(SOURCES[file]).map(
        (h) => `${file.replace("../", "")}:${h.line}`,
      ),
    );
}

describe("no horizontal scrolling, ever", () => {
  it("every scroll container pairs its axis with overflow-x-hidden", () => {
    expect(offenders()).toEqual([]);
  });

  /**
   * A guard that passes on first run proves nothing. This pins the
   * DISCRIMINATION rather than the tree, so a later refactor of the detector
   * cannot quietly make the sweep vacuous.
   */
  it("reports a bare container and accepts a paired one", () => {
    const bare = `  <div className="min-h-0 flex-1 overflow-y-auto py-1">`;
    expect(findBareScrollContainers(bare)).toHaveLength(1);

    const paired = `  <div className="min-h-0 flex-1 overflow-y-auto ${HORIZONTAL_CLIP} py-1">`;
    expect(findBareScrollContainers(paired)).toEqual([]);

    // The three forms with no live use today are covered too, or "free
    // future-proofing" would be a claim with nothing behind it.
    for (const c of SCROLL_AXIS_CLASSES) {
      expect(findBareScrollContainers(`<div className="${c}">`)).toHaveLength(1);
    }
  });

  /**
   * Comments are records, not violations — and the CONTINUATION case is the one
   * a per-line check cannot see. Both middle lines here begin with a backtick,
   * which is exactly the shape that survived the first version of this sweep.
   */
  it("does not report prose describing the rule, including continuation lines", () => {
    const doc = [
      `  {/* The pair, not a bare \`overflow-auto\`: CSS computes an`,
      `      unspecified \`overflow-x\` to \`auto\` when the other axis is`,
      `      non-visible, so \`overflow-y-auto\` alone scrolls sideways. */}`,
      `  <div className="flex-1" />`,
    ].join("\n");
    expect(findBareScrollContainers(doc)).toEqual([]);
  });

  /**
   * The line number must survive the comment strip — round 5's N5, where
   * `stripComments` deleted block comments newline-and-all and every caller's
   * reported line drifted upward by the height of each block above the hit.
   */
  it("reports the real line number underneath a multi-line comment", () => {
    const src = [
      `const a = 1;`,
      `/* a block`,
      `   spanning`,
      `   three lines */`,
      `<div className="overflow-y-auto" />`,
    ].join("\n");
    expect(findBareScrollContainers(src)).toEqual([
      { line: 5, text: `<div className="overflow-y-auto" />` },
    ]);
  });

  /**
   * The exclusion must stay honest — the rule `retired_identifier_test.rs` makes
   * of its own `EXEMPT` list. If `overflow.ts` ever stops spelling the class
   * names, it stops needing the carve-out and the carve-out must go.
   */
  it("keeps its one exclusion earning its place", () => {
    // Looked up by suffix, not by a hardcoded glob key: the key shape depends
    // on the glob's base and a literal would break silently on any move.
    const key = Object.keys(SOURCES).find((k) => k.endsWith("/overflow.ts"));
    expect(key, "the detector module must be in the swept set").toBeDefined();
    const body = SOURCES[key as string];
    expect(
      SCROLL_AXIS_CLASSES.every((c) => body.includes(c)),
      "overflow.ts is exempt only because it defines the class list",
    ).toBe(true);
    expect(EXEMPT_FILES).toEqual(["overflow.ts"]);
  });

  /** The sweep must actually be reading files, not an empty directory. */
  it("reads the frontend source tree", () => {
    expect(Object.keys(SOURCES).filter((p) => !isExempt(p)).length).toBeGreaterThan(40);
  });
});
