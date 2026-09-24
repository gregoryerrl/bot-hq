import { describe, expect, it } from "vitest";
import type { AgentMessage } from "./bindings";
import { compactRows, PASS_NOTICE } from "./chatRows";

let nextId = 1;
function row(author: string, kind: string, content: string): AgentMessage {
  const id = nextId++;
  return { id, session_id: "s1", author, kind, content, created_at: `2026-09-25T00:00:${String(id).padStart(2, "0")}Z` };
}
const passCall = (author: string, id: string) =>
  row(author, "tool_use", JSON.stringify({ input: {}, name: "mcp__bot-hq-signaling__pass_turn", tool_use_id: id }));
const passResult = (author: string, id: string) =>
  row(author, "tool_result", JSON.stringify({ content: '[{"text":"pass noted — your turn is recorded"}]', tool_use_id: id }));

describe("compactRows (feedback #13)", () => {
  it("drops pass tool rows and folds a run of passes into one row", () => {
    const msgs = [
      row("hands", "text", "Committed C1."),
      passCall("eyes", "t1"),
      passResult("eyes", "t1"),
      row("eyes", "text", PASS_NOTICE),
      passCall("hands", "t2"),
      passResult("hands", "t2"),
      row("hands", "text", PASS_NOTICE),
      row("user", "text", "next task"),
    ];
    const rows = compactRows(msgs);
    expect(rows.map((r) => r.kind)).toEqual(["message", "passes", "message"]);
    const passes = rows[1];
    expect(passes.kind === "passes" && passes.authors).toEqual(["eyes", "hands"]);
  });

  it("keeps every other tool row, and a 'pass noted' result that answers no pass call", () => {
    const msgs = [
      row("hands", "tool_use", JSON.stringify({ input: { command: "ls" }, name: "Bash", tool_use_id: "b1" })),
      row("hands", "tool_result", JSON.stringify({ content: "pass noted in the log", tool_use_id: "b1" })),
    ];
    expect(compactRows(msgs).map((r) => r.kind)).toEqual(["message", "message"]);
  });

  it("a pass overridden by prose keeps the prose and hides only the call", () => {
    const msgs = [passCall("eyes", "t3"), passResult("eyes", "t3"), row("eyes", "text", "A real review.")];
    const rows = compactRows(msgs);
    expect(rows).toHaveLength(1);
    expect(rows[0].kind === "message" && rows[0].message.content).toBe("A real review.");
  });
});
