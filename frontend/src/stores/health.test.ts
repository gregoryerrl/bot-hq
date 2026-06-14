import { describe, it, expect } from "vitest";
import { worstHealth, appHealthSummary } from "./health";

describe("worstHealth", () => {
  it("prioritizes dead > retrying > running", () => {
    expect(worstHealth({ brian: "dead", rain: "running" })).toBe("dead");
    expect(worstHealth({ brian: "running", rain: "dead" })).toBe("dead");
    expect(worstHealth({ brian: "retrying", rain: "running" })).toBe("retrying");
    expect(worstHealth({ brian: "running", rain: "running" })).toBe("running");
  });

  it("returns undefined when there is no health data", () => {
    expect(worstHealth(undefined)).toBeUndefined();
    expect(worstHealth({})).toBeUndefined();
  });
});

describe("appHealthSummary", () => {
  it("is ok when nothing is retrying or dead", () => {
    expect(appHealthSummary({})).toEqual({ state: "ok", count: 0 });
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
