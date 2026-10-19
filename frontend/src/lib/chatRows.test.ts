import { describe, expect, it } from "vitest";
import type { AgentMessage } from "./bindings";
import { burstFailures, burstTools, chatRows, compactRows, firstLine, formatSpan, PASS_NOTICE } from "./chatRows";

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

// The pump's shapes: `{input, name, tool_use_id}` and `{content, is_error,
// tool_use_id}`, an MCP tool's content being a JSON string of blocks.
const use = (author: string, id: string, name: string, input: Record<string, unknown>) =>
  row(author, "tool_use", JSON.stringify({ input, name, tool_use_id: id }));
const result = (author: string, id: string, content: string, isError = false) =>
  row(author, "tool_result", JSON.stringify({ content, is_error: isError, tool_use_id: id }));

describe("chatRows — tool bursts (visual pass S3)", () => {
  it("folds one participant's consecutive tool rows into one burst, each call with its result", () => {
    const msgs = [
      row("hands", "text", "Checking the tree."),
      use("hands", "a", "Bash", { command: "git status", description: "Show working tree status" }),
      result("hands", "a", "On branch main\nnothing to commit"),
      use("hands", "b", "Read", { file_path: "/Users/me/p/frontend/src/lib/chatRows.ts" }),
      use("hands", "c", "mcp__bot-hq-signaling__session_doc_write", { slug: "apply" }),
      result("hands", "b", "1\timport type { AgentMessage }"),
      result("hands", "c", JSON.stringify([{ text: '{"id":1,"slug":"apply"}', type: "text" }])),
      row("hands", "text", "Done."),
    ];
    const rows = chatRows(msgs);
    expect(rows.map((r) => r.kind)).toEqual(["message", "burst", "message"]);
    const burst = rows[1];
    if (burst.kind !== "burst") throw new Error("not a burst");
    expect(burst.author).toBe("hands");
    expect(burst.key).toBe(msgs[1].id);
    expect(burst.calls.map((c) => [c.tool, c.label])).toEqual([
      ["Bash", "Show working tree status"],
      ["Read", "Read lib/chatRows.ts"],
      ["session_doc_write", "session_doc_write apply"],
    ]);
    expect(burst.calls.map((c) => firstLine(c.text))).toEqual([
      "On branch main",
      "1\timport type { AgentMessage }",
      '{"id":1,"slug":"apply"}',
    ]);
    expect(burst.firstAt).toBe(msgs[1].created_at);
    expect(burst.lastAt).toBe(msgs[6].created_at);
    expect(burstTools(burst.calls)).toBe("Bash, Read, session_doc_write");
    // The status-line and pass views are unchanged: compactRows keeps the raw rows.
    expect(compactRows(msgs)).toHaveLength(msgs.length);
  });

  it("breaks a burst at text, at a notice and at another participant", () => {
    const msgs = [
      use("hands", "d", "Bash", { command: "ls" }),
      result("hands", "d", "a\nb"),
      row("user", "system_notice", "HANDS's context is at 91%"),
      use("hands", "e", "Bash", { command: "pwd" }),
      result("hands", "e", "/x"),
      use("eyes", "f", "Read", { file_path: "/x/y.ts" }),
      result("eyes", "f", "y"),
    ];
    expect(chatRows(msgs).map((r) => (r.kind === "burst" ? `burst:${r.author}:${r.calls.length}` : r.kind))).toEqual([
      "burst:hands:1",
      "message",
      "burst:hands:1",
      "burst:eyes:1",
    ]);
  });

  it("lets a result that lands after a notice join its own call", () => {
    const msgs = [
      use("hands", "g", "Bash", { command: "cargo build --release" }),
      row("user", "system_notice", "[bot-hq] eyes's context was compacted"),
      result("hands", "g", "Finished `release` profile"),
    ];
    const rows = chatRows(msgs);
    expect(rows.map((r) => r.kind)).toEqual(["burst", "message"]);
    const burst = rows[0];
    if (burst.kind !== "burst") throw new Error("not a burst");
    expect(burst.calls).toHaveLength(1);
    expect(burst.calls[0].result).toBe(msgs[2]);
    expect(burst.lastAt).toBe(msgs[2].created_at);
  });

  it("counts failed calls and keeps a call without its result as running", () => {
    const msgs = [
      use("hands", "h", "Bash", { command: "npx tsc --noEmit" }),
      result("hands", "h", "Exit code 2\nsrc/x.ts(1,1): error TS2304", true),
      use("hands", "i", "Bash", { command: "npx vitest run" }),
    ];
    const burst = chatRows(msgs)[0];
    if (burst.kind !== "burst") throw new Error("not a burst");
    expect(burstFailures(burst.calls)).toBe(1);
    expect(burst.calls[0].failed).toBe(true);
    expect(firstLine(burst.calls[0].text)).toBe("Exit code 2");
    expect(burst.calls[1].result).toBeNull();
    expect(burstTools(burst.calls)).toBe("Bash ×2");
  });

  it("turns an image block into a data URL and never into text", () => {
    const data = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";
    const msgs = [
      use("hands", "j", "mcp__chrome-devtools__take_screenshot", {}),
      result(
        "hands",
        "j",
        JSON.stringify([
          { text: "Took a screenshot of the current page's viewport.", type: "text" },
          { source: { data, media_type: "image/png", type: "base64" }, type: "image" },
        ]),
      ),
    ];
    const burst = chatRows(msgs)[0];
    if (burst.kind !== "burst") throw new Error("not a burst");
    expect(burst.calls[0].images).toEqual([`data:image/png;base64,${data}`]);
    expect(burst.calls[0].text).toBe("Took a screenshot of the current page's viewport.");
    expect(burst.calls[0].text).not.toContain(data);
  });

  it("draws only a raster image, and names any other image type", () => {
    const msgs = [
      use("hands", "s1", "some_tool", {}),
      result(
        "hands",
        "s1",
        JSON.stringify([
          { source: { data: "PHN2Zz48L3N2Zz4=", media_type: "image/svg+xml", type: "base64" }, type: "image" },
          { source: { data: "R0lGODlhAQABAAAAACw=", media_type: "image/gif", type: "base64" }, type: "image" },
        ]),
      ),
    ];
    const burst = chatRows(msgs)[0];
    if (burst.kind !== "burst") throw new Error("not a burst");
    expect(burst.calls[0].images).toEqual(["data:image/gif;base64,R0lGODlhAQABAAAAACw="]);
    expect(burst.calls[0].text).toBe("[image/svg+xml image not shown]");
  });

  it("gives a result whose call is not on screen a call of its own", () => {
    const msgs = [result("hands", "zz", "late output"), use("hands", "k", "Bash", { command: "ls" })];
    const burst = chatRows(msgs)[0];
    if (burst.kind !== "burst") throw new Error("not a burst");
    expect(burst.calls.map((c) => [c.tool, c.use === null, c.text])).toEqual([
      ["result", true, "late output"],
      ["Bash", false, ""],
    ]);
  });

  it("still drops pass rows and folds pass lines", () => {
    const msgs = [passCall("eyes", "p1"), passResult("eyes", "p1"), row("eyes", "text", PASS_NOTICE)];
    expect(chatRows(msgs).map((r) => r.kind)).toEqual(["passes"]);
  });

  it("names at most four tools, then how many more", () => {
    const calls = ["Bash", "Read", "Edit", "Grep", "Write", "Glob"].map((name, i) =>
      use("hands", `t${i}`, name, {}),
    );
    const burst = chatRows([...calls, use("hands", "t9", "Bash", {})])[0];
    if (burst.kind !== "burst") throw new Error("not a burst");
    expect(burstTools(burst.calls)).toBe("Bash ×2, Read, Edit, Grep, +2 more");
  });

  it("formats a burst's span", () => {
    expect(formatSpan(400)).toBe("0s");
    expect(formatSpan(14_200)).toBe("14s");
    expect(formatSpan(125_000)).toBe("2m 05s");
    expect(formatSpan(3_780_000)).toBe("1h 03m");
  });
});
