import { describe, expect, it } from "vitest";
import {
  BURST_THRESHOLD,
  COOLDOWN_MS,
  URGENT_RENOTIFY_MAX,
  URGENT_RENOTIFY_MS,
  nextUrgentDelay,
  planFlush,
  urgentToast,
  type EscalationEvent,
} from "./osNotifications";

// Feedback #52: the urgent gate's toast and its repeat schedule.
describe("urgent gates", () => {
  const gate = {
    choiceId: "694e9824-x",
    sessionId: "s-1",
    agent: "hands",
    reason: "Cloud Run job is burning the shared Meta budget",
    command: "\n  gcloud run jobs executions cancel nightly --region us-east1\nsecond line",
  };

  it("leads with the reason, then the command's first line", () => {
    const t = urgentToast(gate, 0);
    expect(t.title).toBe("bot-hq — URGENT approval");
    expect(t.body).toBe(
      "Cloud Run job is burning the shared Meta budget — gcloud run jobs executions cancel nightly --region us-east1",
    );
    expect(urgentToast({ ...gate, command: "" }, 0).body).toBe(gate.reason);
  });

  it("counts the repeats in the title", () => {
    expect(urgentToast(gate, 3).title).toBe(`bot-hq — URGENT approval (still waiting, 3/${URGENT_RENOTIFY_MAX})`);
  });

  // Feedback #127: a forced approval notifies while the reviewer still has to
  // read it — the toast says so instead of claiming an approval is waiting.
  it("says the gate is under review when it is not yet the user's to answer", () => {
    const t = urgentToast({ ...gate, underReview: true }, 0);
    expect(t.title).toBe("bot-hq — URGENT, under review");
    expect(t.body).toBe(
      "reviewer reads it first — Cloud Run job is burning the shared Meta budget — gcloud run jobs executions cancel nightly --region us-east1",
    );
    expect(urgentToast({ ...gate, underReview: true }, 2).title).toBe(
      `bot-hq — URGENT, under review (still waiting, 2/${URGENT_RENOTIFY_MAX})`,
    );
    expect(urgentToast({ ...gate, underReview: false }, 0).title).toBe("bot-hq — URGENT approval");
  });

  it("repeats every ten minutes, six times, then stops", () => {
    expect(URGENT_RENOTIFY_MS).toBe(600_000);
    expect(URGENT_RENOTIFY_MAX).toBe(6);
    for (let sent = 0; sent < URGENT_RENOTIFY_MAX; sent++) {
      expect(nextUrgentDelay(sent)).toBe(URGENT_RENOTIFY_MS);
    }
    expect(nextUrgentDelay(URGENT_RENOTIFY_MAX)).toBeNull();
  });
});

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
