import { describe, it, expect } from "vitest";
import { authorColorClass } from "./authorColor";
import { UNKNOWN_PARTICIPANT } from "../lib/participants";

describe("authorColorClass", () => {
  it("tints a role-derived participant instead of dropping it to neutral", () => {
    // The regression this fixes: the map held two agent slugs, so once slugs
    // became role-derived (rc3 D10) EVERY participant missed it and fell
    // through to the neutral tone — bylines lost their per-participant colour.
    expect(authorColorClass("HANDS · Claude Opus 5")).not.toBe(
      "text-on-surface-variant",
    );
    expect(authorColorClass("EYES · DeepSeek R2")).not.toBe(
      "text-on-surface-variant",
    );
  });

  it("gives one participant ONE colour, on every surface and every run", () => {
    // The chat byline holds a slug and the turn-status line holds a slot key;
    // the resolved label is the only string they share, so keying on it is
    // what makes the two agree. Stability is the whole requirement.
    const label = "EYES · Claude Opus 5";
    expect(authorColorClass(label)).toBe(authorColorClass(label));
  });

  it("keeps the non-participant authors on their own reserved tones", () => {
    expect(authorColorClass("You")).toBe("text-author-user");
    expect(authorColorClass("System")).toBe("text-on-surface-variant");
  });

  it("leaves an unplaceable author neutral rather than hueing the crowd", () => {
    // Every author the roster cannot place shares this one label, so a hue
    // would imply they are all the same participant.
    expect(authorColorClass(UNKNOWN_PARTICIPANT)).toBe(
      "text-on-surface-variant",
    );
    expect(authorColorClass("")).toBe("text-on-surface-variant");
  });

  it("returns a class string for a label that collides with Object.prototype", () => {
    // A bare object lookup answers `toString` / `constructor` out of the
    // prototype and hands a FUNCTION back as a className.
    for (const label of ["toString", "constructor", "valueOf", "__proto__"]) {
      expect(typeof authorColorClass(label)).toBe("string");
      expect(authorColorClass(label)).toMatch(/^text-/);
    }
  });

  it("only ever answers with a colour token the palette actually defines", () => {
    // `author-brian` / `author-rain` are palette TOKEN names in
    // tailwind.config.ts, not agent identities — nothing renders them. A typo
    // here is invisible at runtime (Tailwind just emits no rule).
    const DEFINED = [
      "text-author-brian",
      "text-author-rain",
      "text-author-user",
      "text-on-surface-variant",
    ];
    const labels = [
      "HANDS · Claude Opus 5",
      "EYES · DeepSeek R2",
      "EYES · Claude Opus 5",
      "You",
      "System",
      UNKNOWN_PARTICIPANT,
      "",
    ];
    for (const l of labels) expect(DEFINED).toContain(authorColorClass(l));
  });
});
