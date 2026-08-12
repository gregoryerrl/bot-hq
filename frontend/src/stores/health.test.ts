import { describe, it, expect } from "vitest";
import { worstHealth, appHealthSummary } from "./health";

describe("worstHealth", () => {
  it("prioritizes dead > retrying > running", () => {
    expect(worstHealth({ brian: "dead", rain: "running" })).toBe("dead");
    expect(worstHealth({ brian: "running", rain: "dead" })).toBe("dead");
    expect(worstHealth({ brian: "retrying", rain: "running" })).toBe("retrying");
    expect(worstHealth({ brian: "running", rain: "running" })).toBe("running");
  });

  it("ranks every tier against its neighbour", () => {
    // The pairwise cases the tier list actually encodes. `dead` vs `stalled`
    // was untested, so an inverted order between them passed silently.
    expect(worstHealth({ a: "dead", b: "stalled" })).toBe("dead");
    expect(worstHealth({ a: "stalled", b: "retrying" })).toBe("stalled");
    expect(worstHealth({ a: "retrying", b: "running" })).toBe("retrying");
  });

  it("scans every agent, not just the first two", () => {
    // The map is keyed by participant slug and a session can hold more than
    // two, so a third agent's state has to count.
    expect(worstHealth({ one: "running", two: "running", three: "dead" })).toBe(
      "dead",
    );
  });

  it("returns undefined when there is no health data", () => {
    expect(worstHealth(undefined)).toBeUndefined();
    expect(worstHealth({})).toBeUndefined();
  });
});

describe("appHealthSummary", () => {
  it("is idle when no agents are tracked", () => {
    expect(appHealthSummary({})).toEqual({ state: "idle", count: 0 });
  });

  it("is ok when something is running but nothing is retrying or dead", () => {
    expect(appHealthSummary({ a: { brian: "running" } })).toEqual({
      state: "ok",
      count: 0,
    });
  });

  it("counts dead sessions and dead wins over retrying", () => {
    const r = appHealthSummary({
      a: { brian: "dead" },
      b: { brian: "retrying" },
      c: { brian: "running", rain: "dead" },
    });
    expect(r).toEqual({ state: "dead", count: 2 });
  });

  it("reports retrying when no session is dead", () => {
    expect(
      appHealthSummary({ a: { rain: "retrying" }, b: { brian: "running" } }),
    ).toEqual({ state: "retrying", count: 1 });
  });
});
