import { describe, it, expect } from "vitest";
import { anyBusy, isLocked } from "./activity";

/**
 * `isLocked` is one boolean, and the entire interrupt model rests on it (rc3
 * D33): **you cannot type while agents work, and Pause is the only interrupt.**
 * It had no unit test until the rule became load-bearing — the behaviour was
 * only ever asserted through `ChatInput`, which meant every case had to be
 * expressed as a render.
 */
describe("isLocked", () => {
  it("locks whenever a participant is mid-turn", () => {
    expect(isLocked("busy", { hands: true })).toBe(true);
    expect(isLocked("busy")).toBe(true);
  });

  it("locks on the busy MAP when the collapsed enum has lost it", () => {
    // `SessionActivity::derive` ranks `awaiting` ABOVE `busy`, so parking a
    // question reports `awaiting_user` while participants run. Trusting the
    // enum alone is what put an open textarea over a working session.
    expect(isLocked("awaiting_user", { hands: true, eyes: false })).toBe(true);
    expect(isLocked("idle", { eyes: true })).toBe(true);
  });

  it("opens the moment the last participant stops", () => {
    // "No halt = no type" is a floor, not a ceiling. Nothing has to GRANT the
    // box back; it returns when nobody is working.
    expect(isLocked("awaiting_user", { hands: false, eyes: false })).toBe(false);
    expect(isLocked("idle", {})).toBe(false);
    expect(isLocked("idle")).toBe(false);
  });

  it("gives the box to a paused session even mid-drain", () => {
    // Pause is the ONE interrupt, and this is the whole reason to press it.
    // Agents are stopped by the time it lands; the busy flags may still be
    // unwinding a tool call. Withholding the box here would make the only
    // interrupt in the product fail to do the thing it exists for.
    expect(isLocked("paused", { hands: true })).toBe(false);
    expect(isLocked("paused")).toBe(false);
  });

  it("holds the box through a cancel in flight", () => {
    // Between the press and the stop the outcome is unknown, so the box would
    // flicker open and shut. `cancelling` is the only state that outranks the
    // busy map in the other direction.
    expect(isLocked("cancelling")).toBe(true);
    expect(isLocked("cancelling", {})).toBe(true);
  });

  it("treats an unseen session as idle", () => {
    // No `session:activity` event yet — a session the user just opened. Locking
    // by default would leave the box shut on a session that never starts one.
    expect(isLocked(undefined)).toBe(false);
  });
});

describe("anyBusy", () => {
  it("is false for absent, empty, and all-false maps", () => {
    expect(anyBusy(undefined)).toBe(false);
    expect(anyBusy({})).toBe(false);
    expect(anyBusy({ hands: false, eyes: false })).toBe(false);
  });

  it("is true if any single participant is working", () => {
    expect(anyBusy({ hands: false, eyes: true })).toBe(true);
  });
});
