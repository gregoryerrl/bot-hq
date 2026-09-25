import { describe, it, expect } from "vitest";
import { needsYou, needsYouBySession, needsYouParts, wakesLabel } from "./attention";

// The user, 2026-09-25: gates and halts showed on neither the dashboard cards
// nor the header bell. Each kind is named on its own — a blended count is what
// rc3 D35 removed them for.
describe("needsYouBySession", () => {
  const question = (sid: string) => ({ session_id: sid, kind: "choice", options: ["A", "B"] });
  const gate = (sid: string) => ({ session_id: sid, kind: "approval", options: ["Approve", "Reject"] });
  const halt = (sid: string, wake_at: string | null = null) => ({
    session_id: sid,
    declared_by: "hands",
    reason: "your move",
    wake_at,
  });
  const NOW = Date.parse("2026-09-25T05:00:00Z");

  it("counts questions and approval gates apart, per session", () => {
    const n = needsYouBySession([question("a"), question("a"), gate("a"), gate("b")], [], NOW);
    expect(n.a).toEqual({ questions: 2, approvals: 1, halt: null, wakesAt: null });
    expect(n.b.approvals).toBe(1);
    expect(needsYouParts(n.a)).toEqual(["approval waiting", "2 questions"]);
  });

  it("names a halt as the user's move, with its reason", () => {
    const n = needsYouBySession([], [halt("a")], NOW);
    expect(n.a.halt).toEqual({ reason: "your move", declaredBy: "hands" });
    expect(needsYou(n.a)).toBe(true);
    expect(needsYouParts(n.a)).toEqual(["halted — your move"]);
  });

  it("does not count a temporary halt still ahead of its wake time", () => {
    const n = needsYouBySession([], [halt("a", "2026-09-25T05:30:00Z")], NOW);
    expect(needsYou(n.a)).toBe(false);
    expect(n.a.wakesAt).toBe("2026-09-25T05:30:00Z");
  });

  it("counts a temporary halt whose wake time has passed (EYES Q1)", () => {
    // The wake is held while the session is paused, stopped or behind a gate —
    // past its time, the session is waiting on the user.
    const n = needsYouBySession([], [halt("a", "2026-09-25T04:30:00Z")], NOW);
    expect(needsYou(n.a)).toBe(true);
    expect(n.a.halt?.reason).toBe("your move");
    expect(n.a.wakesAt).toBeNull();
  });

  it("ignores a legacy halt tray row", () => {
    const n = needsYouBySession([{ session_id: "a", kind: "halt", options: [] }], [], NOW);
    expect(needsYou(n.a)).toBe(false);
  });

  it("formats a wake time on the local clock", () => {
    const at = new Date(2026, 8, 25, 14, 5).toISOString();
    expect(wakesLabel(at)).toBe("wakes 14:05");
  });
});
