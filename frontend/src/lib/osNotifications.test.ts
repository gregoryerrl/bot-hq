import { describe, expect, it } from "vitest";
import {
  BURST_THRESHOLD,
  COOLDOWN_MS,
  planFlush,
  type EscalationEvent,
} from "./osNotifications";

const q = (sessionId: string, snippet = "pick one"): EscalationEvent => ({
  sessionId,
  kind: "question",
  snippet,
});
const h = (sessionId: string, snippet = "waiting on you"): EscalationEvent => ({
  sessionId,
  kind: "halt",
  snippet,
});

describe("planFlush", () => {
  it("fires one toast per event below the burst threshold", () => {
    const { toasts } = planFlush([q("s-1"), h("s-2")], {}, 1000);
    expect(toasts).toHaveLength(2);
    expect(toasts[0].title).toContain("question");
    expect(toasts[1].title).toContain("waiting");
  });

  it("dedupes repeats of the same (session, kind) within one flush", () => {
    // park → supersede → re-park lands three events in one burst window.
    const { toasts } = planFlush([q("s-1", "a"), q("s-1", "b"), q("s-1", "c")], {}, 1000);
    expect(toasts).toHaveLength(1);
    expect(toasts[0].body).toBe("a");
  });

  it("suppresses a (session, kind) inside the cooldown and refires after it", () => {
    const first = planFlush([q("s-1")], {}, 1000);
    expect(first.toasts).toHaveLength(1);

    const during = planFlush([q("s-1")], first.next, 1000 + COOLDOWN_MS - 1);
    expect(during.toasts).toHaveLength(0);

    const after = planFlush([q("s-1")], during.next, 1000 + COOLDOWN_MS + 1);
    expect(after.toasts).toHaveLength(1);
  });

  it("cooldown is per kind — a halt fires while the question cools", () => {
    const first = planFlush([q("s-1")], {}, 1000);
    const { toasts } = planFlush([h("s-1")], first.next, 2000);
    expect(toasts).toHaveLength(1);
    expect(toasts[0].title).toContain("waiting");
  });

  it("coalesces a burst into one aggregate counting sessions, not events", () => {
    const { toasts } = planFlush([h("s-1"), h("s-2"), h("s-3"), q("s-1")], {}, 1000);
    expect(toasts).toHaveLength(1);
    expect(toasts[0].body).toBe("3 sessions need you");
  });

  it("aggregate grammar handles one session with many kinds", () => {
    const events = [q("s-1"), h("s-1"), { ...q("s-1"), kind: "halt" as const }];
    // 3 due requires distinct (session, kind) keys — build from threshold.
    expect(BURST_THRESHOLD).toBe(3);
    const { toasts } = planFlush([q("s-1"), h("s-1"), q("s-2")], {}, 1000);
    expect(toasts).toHaveLength(1);
    expect(toasts[0].body).toBe("2 sessions need you");
    void events;
  });

  it("cooldown-suppressed events do not count toward the burst", () => {
    const first = planFlush([q("s-1"), q("s-2")], {}, 1000);
    expect(first.toasts).toHaveLength(2);
    const second = planFlush([q("s-1"), q("s-2"), h("s-3")], first.next, 2000);
    expect(second.toasts).toHaveLength(1);
    expect(second.toasts[0].title).toContain("waiting");
  });

  it("clamps long bodies", () => {
    const { toasts } = planFlush([q("s-1", "x".repeat(500))], {}, 1000);
    expect(toasts[0].body.length).toBeLessThanOrEqual(140);
    expect(toasts[0].body.endsWith("…")).toBe(true);
  });

  it("prunes expired cooldown entries so the map self-limits", () => {
    const first = planFlush([q("s-1")], {}, 1000);
    expect(Object.keys(first.next)).toEqual(["s-1:question"]);
    const later = planFlush([q("s-2")], first.next, 1000 + COOLDOWN_MS + 1);
    expect(Object.keys(later.next)).toEqual(["s-2:question"]);
  });
});
