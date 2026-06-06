import { describe, it, expect } from "vitest";
import { phaseBucket } from "./phase";

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
