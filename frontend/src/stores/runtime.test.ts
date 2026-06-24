import { describe, it, expect, vi } from "vitest";
import { seedRuntimeStores, type SessionRuntime } from "./runtime";

describe("seedRuntimeStores", () => {
  it("seeds activity for every row and health for non-null agents", () => {
    const setActivity = vi.fn();
    const setHealth = vi.fn();
    const rows: SessionRuntime[] = [
      {
        session_id: "s1",
        activity: "busy",
        brian_health: "running",
        rain_health: "retrying",
      },
      {
        session_id: "s2",
        activity: "awaiting_user",
        brian_health: "dead",
        rain_health: null,
      },
    ];

    seedRuntimeStores(rows, setActivity, setHealth);

    expect(setActivity).toHaveBeenCalledWith("s1", "busy");
    expect(setActivity).toHaveBeenCalledWith("s2", "awaiting_user");
    expect(setHealth).toHaveBeenCalledWith("s1", "brian", "running");
    expect(setHealth).toHaveBeenCalledWith("s1", "rain", "retrying");
    expect(setHealth).toHaveBeenCalledWith("s2", "brian", "dead");
    // s2.rain_health is null → no setHealth call for it.
    expect(setHealth).not.toHaveBeenCalledWith("s2", "rain", expect.anything());
    expect(setHealth).toHaveBeenCalledTimes(3);
  });

  it("is a no-op for an empty snapshot", () => {
    const setActivity = vi.fn();
    const setHealth = vi.fn();
    seedRuntimeStores([], setActivity, setHealth);
    expect(setActivity).not.toHaveBeenCalled();
    expect(setHealth).not.toHaveBeenCalled();
  });
});
