import { describe, it, expect, beforeEach } from "vitest";
import { useTrayStaging, stagedFor } from "./trayStaging";

describe("trayStaging (rc3 D34)", () => {
  beforeEach(() => {
    useTrayStaging.setState({ staged: {} });
  });

  it("stages, re-stages, and unstages per session", () => {
    const s = useTrayStaging.getState();
    s.stage("s1", "c-1", "Yes");
    s.stage("s1", "c-2", "main");
    s.stage("s1", "c-1", "No"); // a staged pick is a draft — re-picking overwrites
    expect(stagedFor(useTrayStaging.getState().staged, "s1")).toEqual({
      "c-1": "No",
      "c-2": "main",
    });
    s.unstage("s1", "c-2");
    expect(stagedFor(useTrayStaging.getState().staged, "s1")).toEqual({
      "c-1": "No",
    });
  });

  it("clears one session without touching another", () => {
    const s = useTrayStaging.getState();
    s.stage("s1", "c-1", "Yes");
    s.stage("s2", "c-9", "No");
    s.clear("s1");
    expect(stagedFor(useTrayStaging.getState().staged, "s1")).toEqual({});
    expect(stagedFor(useTrayStaging.getState().staged, "s2")).toEqual({
      "c-9": "No",
    });
  });

  it("returns a stable empty object for a session with nothing staged", () => {
    const a = stagedFor(useTrayStaging.getState().staged, "s1");
    const b = stagedFor(useTrayStaging.getState().staged, "s1");
    expect(a).toBe(b); // referential stability — this feeds a useMemo
  });
});
