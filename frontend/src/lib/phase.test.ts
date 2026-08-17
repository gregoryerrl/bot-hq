import { describe, it, expect } from "vitest";
import { phaseBucket, PHASE_NAMES, isPhaseName } from "./phase";

describe("phaseBucket", () => {
  it("maps each IPAV phase to its color bucket", () => {
    expect(phaseBucket("investigate")).toBe("primary");
    expect(phaseBucket("plan")).toBe("primary");
    expect(phaseBucket("apply")).toBe("secondary");
    expect(phaseBucket("verify")).toBe("tertiary");
  });

  it("is case-insensitive (the chip passes a raw phase string)", () => {
    expect(phaseBucket("INVESTIGATE")).toBe("primary");
    expect(phaseBucket("Verify")).toBe("tertiary");
  });

  it("returns null for unknown / done phases", () => {
    expect(phaseBucket("done")).toBeNull();
    expect(phaseBucket("")).toBeNull();
    expect(phaseBucket("whatever")).toBeNull();
  });
});

describe("PHASE_NAMES", () => {
  it("is the one IPAV set the select, the type and the tints derive from (round 8)", () => {
    expect([...PHASE_NAMES]).toEqual(["investigate", "plan", "apply", "verify"]);
    for (const p of PHASE_NAMES) expect(phaseBucket(p)).not.toBeNull();
    expect(isPhaseName("Apply")).toBe(true);
    expect(isPhaseName("done")).toBe(false);
    expect(phaseBucket("done")).toBeNull();
  });
});
