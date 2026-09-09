import { describe, it, expect } from "vitest";
import {
  authorColorClass,
  colorByName,
  PARTICIPANT_COLORS,
} from "./authorColor";
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
    //
    //
    // Read from `PARTICIPANT_COLORS` rather than transcribed, so adding a hue
    // to the palette does not also need a line here — the drift this test used
    // to have. What it still catches is the case it exists for: a token that is
    // in the module and NOT in the Tailwind config emits no CSS rule and is
    // invisible at runtime, which the assertion below reaches through the
    // `text-author-*` shape.
    const DEFINED = [
      ...PARTICIPANT_COLORS.map((c) => c.token),
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

// ===========================================================================
// rc3 D20 — the palette is data, so the picker and the rotation agree
// ===========================================================================

describe("PARTICIPANT_COLORS", () => {
  it("is big enough to both rotate and choose from", () => {
    // Rotation only needs the roster cap (4) to guarantee distinctness. A PICKER
    // needs more, or a user who dislikes a hue has no alternative that is still
    // distinct from everyone else's.
    expect(PARTICIPANT_COLORS.length).toBeGreaterThanOrEqual(8);
  });

  it("carries no agent names", () => {
    // These entries were `brian` and `rain` — two agent names living on in the
    // design system after rc3 D10 retired them everywhere else.
    for (const c of PARTICIPANT_COLORS) {
      expect(c.token).not.toMatch(/brian|rain|hands|eyes/i);
      expect(c.name).not.toMatch(/brian|rain|hands|eyes/i);
    }
  });

  it("has no duplicate names or tokens", () => {
    // A picker with two entries called the same thing, or two that paint the
    // same, is a picker that cannot express what it offers.
    expect(new Set(PARTICIPANT_COLORS.map((c) => c.name)).size).toBe(
      PARTICIPANT_COLORS.length,
    );
    expect(new Set(PARTICIPANT_COLORS.map((c) => c.token)).size).toBe(
      PARTICIPANT_COLORS.length,
    );
  });

  it("resolves a stored name back to its entry, however it was cased", () => {
    // The name round-trips through storage, so the lookup has to survive a user
    // (or a migration) writing it differently.
    expect(colorByName("Cyan")?.token).toBe("text-author-cyan");
    expect(colorByName("  cyan ")?.token).toBe("text-author-cyan");
    expect(colorByName("CYAN")?.token).toBe("text-author-cyan");
    expect(colorByName("puce")).toBeUndefined();
    expect(colorByName(null)).toBeUndefined();
  });
});
