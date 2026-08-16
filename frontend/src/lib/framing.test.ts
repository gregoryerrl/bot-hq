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
    expect(findRetiredFraming("if (!session.rain_enabled) return null;")).toEqual(
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
   * Two files, each for a reason that is a property of the file rather than a
   * judgement about its contents — which is what keeps this from being the
   * sanctioned-absence list that rots into permission for the next one.
   *
   * - `bindings.ts` is GENERATED from Rust at app launch and is `@ts-nocheck`.
   *   Its content is the wire, which keeps the retired names deliberately (the
   *   external driver's fields); an edit here is overwritten on next launch.
   * - `framing.ts` DEFINES the pattern, so it necessarily spells the words it
   *   searches for. Building the regex from fragments to slip past its own
   *   check would be rewording around the gate — the exact move the general
   *   rules forbid — so the exemption is stated instead of hidden.
   */
  const EXEMPT = ["bindings.ts", "framing.ts"];

  it("no rendered string names a retired agent or assumes a pair", () => {
    const files = Object.keys(SOURCES).filter(
      (p) =>
        !/\.test\.tsx?$/.test(p) && !EXEMPT.some((e) => p.endsWith(`/${e}`)),
    );
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
