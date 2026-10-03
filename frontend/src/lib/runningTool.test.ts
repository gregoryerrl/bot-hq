import { describe, expect, it } from "vitest";
import type { AgentMessage } from "./bindings";
import { describeToolCall, runningTool } from "./runningTool";

let nextId = 1;
const at = (secondsAgo: number) => new Date(Date.UTC(2026, 9, 3, 5, 0, 0) - secondsAgo * 1000).toISOString();

function row(author: string, kind: string, content: string, secondsAgo: number): AgentMessage {
  return { id: nextId++, session_id: "s1", author, kind, content, created_at: at(secondsAgo) };
}
const use = (author: string, id: string, name: string, input: unknown, secondsAgo: number) =>
  row(author, "tool_use", JSON.stringify({ name, input, tool_use_id: id }), secondsAgo);
const result = (author: string, id: string, secondsAgo: number) =>
  row(author, "tool_result", JSON.stringify({ tool_use_id: id, content: "ok", is_error: false }), secondsAgo);

describe("describeToolCall", () => {
  it("uses the call's own description when it wrote one", () => {
    expect(
      describeToolCall("Bash", { command: "cargo test 2>&1 | tail", description: "Run the full Rust test suite" }),
    ).toBe("Run the full Rust test suite");
    expect(describeToolCall("Agent", { description: "Map action gate lifecycle", prompt: "…" })).toBe(
      "Map action gate lifecycle",
    );
  });

  it("otherwise names the tool and its main argument", () => {
    expect(describeToolCall("Read", { file_path: "/Users/x/Projects/bot-hq/src/core/pump.rs" })).toBe(
      "Read core/pump.rs",
    );
    expect(describeToolCall("Grep", { pattern: "fn render_wire", path: "src" })).toBe("Grep fn render_wire");
    expect(describeToolCall("LS", { path: "/repo/frontend/src" })).toBe("LS frontend/src");
    expect(describeToolCall("mcp__bot-hq-signaling__session_doc_write", { slug: "plan", body: "…" })).toBe(
      "session_doc_write plan",
    );
    expect(describeToolCall("mcp__bot-hq-signaling__pass_turn", {})).toBe("pass_turn");
    expect(describeToolCall("Bash", { command: "git   status\n--short" })).toBe("Bash git status --short");
  });

  it("keeps the line short and survives a missing input", () => {
    const long = describeToolCall("Bash", { description: "x".repeat(300) });
    expect(long.length).toBe(90);
    expect(long.endsWith("…")).toBe(true);
    expect(describeToolCall("Bash", undefined)).toBe("Bash");
    expect(describeToolCall("Bash", { description: "   " })).toBe("Bash");
  });
});

describe("runningTool", () => {
  it("is the newest unanswered call of the participant's current run", () => {
    const messages = [
      row("user", "text", "go", 600),
      row("hands", "text", "Reading the queue.", 590),
      use("hands", "t1", "Bash", { description: "List the open items", command: "sqlite3 …" }, 580),
      result("hands", "t1", 575),
      use("hands", "t2", "Bash", { description: "Run the full Rust test suite", command: "cargo test" }, 120),
    ];
    expect(runningTool(messages, "hands")).toEqual({
      label: "Run the full Rust test suite",
      startedAt: Date.parse(at(120)),
      others: 0,
    });
  });

  it("is nothing once the last call was answered, or while the participant writes", () => {
    const answered = [
      use("hands", "t1", "Bash", { description: "List the open items" }, 60),
      result("hands", "t1", 55),
    ];
    expect(runningTool(answered, "hands")).toBeNull();
    expect(runningTool([...answered, row("hands", "text", "Done.", 50)], "hands")).toBeNull();
    expect(runningTool(undefined, "hands")).toBeNull();
    expect(runningTool([], "hands")).toBeNull();
  });

  it("counts parallel calls and names the newest", () => {
    const messages = [
      use("hands", "a", "Agent", { description: "Map compaction and spawn code" }, 30),
      use("hands", "b", "Agent", { description: "Map action gate lifecycle" }, 29),
      use("hands", "c", "Read", { file_path: "/repo/src/agents/spawn.rs" }, 28),
      result("hands", "b", 20),
    ];
    expect(runningTool(messages, "hands")).toEqual({
      label: "Read agents/spawn.rs",
      startedAt: Date.parse(at(28)),
      others: 1,
    });
  });

  it("stops at another participant's row and skips host notices", () => {
    const messages = [
      use("hands", "old", "Bash", { description: "An earlier turn's call, never answered" }, 900),
      row("eyes", "text", "Reviewed.", 800),
      use("hands", "now", "Bash", { description: "Build the release binary" }, 70),
      row("", "system_notice", "⚠ hands's context is at 86 %", 40),
    ];
    expect(runningTool(messages, "hands")?.label).toBe("Build the release binary");
    // The reviewer's last row is text: it has nothing in flight.
    expect(runningTool(messages.slice(0, 2), "eyes")).toBeNull();
  });

  it("reads a row's id without parsing its payload", () => {
    // A tool result holds a whole tool output; the id is taken from the tail.
    // The payload here is not even valid JSON, so a parse would have thrown.
    const huge = `{"content":"${"x".repeat(50_000)} "unbalanced \\" quote","is_error":false,"tool_use_id":"big"}`;
    const messages = [
      use("hands", "big", "Bash", { description: "Dump the database" }, 30),
      row("hands", "tool_result", huge, 20),
      use("hands", "next", "Bash", { description: "Summarise the dump" }, 10),
    ];
    expect(runningTool(messages, "hands")?.label).toBe("Summarise the dump");
    expect(runningTool(messages.slice(0, 2), "hands")).toBeNull();
  });

  it("ignores rows it cannot read instead of guessing", () => {
    const messages = [
      row("hands", "tool_use", "x", 30),
      row("hands", "tool_use", JSON.stringify({ name: "Bash", input: {} }), 20),
    ];
    expect(runningTool(messages, "hands")).toBeNull();
  });
});
