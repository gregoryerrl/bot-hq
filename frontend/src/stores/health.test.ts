import { describe, it, expect } from "vitest";
import { worstHealth } from "./health";

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
