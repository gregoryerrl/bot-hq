import { describe, expect, it } from "vitest";
import {
  contextFraction,
  contextSeverity,
  formatTokens,
  useContextStore,
} from "./context";

describe("contextFraction", () => {
  it("divides used by window", () => {
    expect(contextFraction({ usedTokens: 619856, contextWindow: 1_000_000 })).toBeCloseTo(
      0.619856,
    );
  });

  // "no entry" means UNKNOWN, not empty — the backend emits nothing when the
  // provider withheld a window, so callers must be able to tell the difference.
  it("returns undefined for a missing entry", () => {
    expect(contextFraction(undefined)).toBeUndefined();
  });

  // Defence in depth: the backend already refuses to emit a zero window, but a
  // NaN reaching a style attribute breaks silently rather than loudly.
  it("returns undefined rather than NaN for a zero or negative window", () => {
    expect(contextFraction({ usedTokens: 10, contextWindow: 0 })).toBeUndefined();
    expect(contextFraction({ usedTokens: 10, contextWindow: -1 })).toBeUndefined();
  });
});

describe("contextSeverity", () => {
  it("bands on action, not aesthetics", () => {
    expect(contextSeverity(0)).toBe("ok");
    expect(contextSeverity(0.69)).toBe("ok");
    expect(contextSeverity(0.7)).toBe("warn");
    expect(contextSeverity(0.89)).toBe("warn");
    expect(contextSeverity(0.9)).toBe("critical");
    expect(contextSeverity(1.2)).toBe("critical");
  });
});

describe("formatTokens", () => {
  it("compacts for the tooltip", () => {
    expect(formatTokens(842)).toBe("842");
    expect(formatTokens(619_856)).toBe("620K");
    expect(formatTokens(1_000_000)).toBe("1.0M");
    expect(formatTokens(200_000)).toBe("200K");
  });
});

describe("useContextStore", () => {
  it("tracks agents independently within a session", () => {
    const { setContext } = useContextStore.getState();
    setContext("s1", "brian", { usedTokens: 100, contextWindow: 1000 });
    setContext("s1", "rain", { usedTokens: 500, contextWindow: 1000 });
    const s1 = useContextStore.getState().bySession.s1;
    expect(s1.brian?.usedTokens).toBe(100);
    expect(s1.rain?.usedTokens).toBe(500);
  });

  // Occupancy DROPS when claude-code auto-compacts. The store must accept a
  // lower value as a normal update, not treat it as stale and keep the peak.
  it("accepts a decrease (compaction is normal, not a regression)", () => {
    const { setContext } = useContextStore.getState();
    setContext("s2", "brian", { usedTokens: 900, contextWindow: 1000 });
    setContext("s2", "brian", { usedTokens: 120, contextWindow: 1000 });
    expect(useContextStore.getState().bySession.s2.brian?.usedTokens).toBe(120);
  });

  it("clears only the named session", () => {
    const { setContext, clearSession } = useContextStore.getState();
    setContext("s3", "brian", { usedTokens: 1, contextWindow: 10 });
    setContext("s4", "brian", { usedTokens: 2, contextWindow: 10 });
    clearSession("s3");
    const after = useContextStore.getState().bySession;
    expect(after.s3).toBeUndefined();
    expect(after.s4?.brian?.usedTokens).toBe(2);
  });

  it("clearing an unknown session is a no-op that preserves identity", () => {
    const before = useContextStore.getState().bySession;
    useContextStore.getState().clearSession("never-existed");
    expect(useContextStore.getState().bySession).toBe(before);
  });
});
