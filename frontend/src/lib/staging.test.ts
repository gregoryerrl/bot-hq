import { describe, expect, it } from "vitest";
import { picksDiffer, stagedKey } from "./staging";

describe("stagedKey", () => {
  it("moves when a staged pick's VALUE changes, not only its count", () => {
    // RED on the round-11 key (`Object.keys(map).length`): same count, new value.
    const before = stagedKey({ q1: "A" });
    const after = stagedKey({ q1: "B" });
    expect(after).not.toBe(before);
    expect(Object.keys({ q1: "A" }).length).toBe(Object.keys({ q1: "B" }).length);
  });
  it("moves on add and remove too, and is order-free", () => {
    expect(stagedKey({})).not.toBe(stagedKey({ q1: "A" }));
    expect(stagedKey({ q1: "A", q2: "B" })).toBe(stagedKey({ q2: "B", q1: "A" }));
  });
});

describe("picksDiffer", () => {
  const a = { choice_id: "q1", picked: "A" };
  it("is false for an identical snapshot", () => {
    expect(picksDiffer([a], [{ ...a }])).toBe(false);
  });
  it("is true when a value changed with the same count", () => {
    expect(picksDiffer([{ choice_id: "q1", picked: "B" }], [a])).toBe(true);
  });
  it("is true on count or choice changes", () => {
    expect(picksDiffer([], [a])).toBe(true);
    expect(picksDiffer([{ choice_id: "q2", picked: "A" }], [a])).toBe(true);
  });
});
