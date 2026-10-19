import { fireEvent, render, screen, within } from "@testing-library/react";
import { useState } from "react";
import type { AgentMessage } from "../lib/bindings";
import { chatRows, type BurstRow } from "../lib/chatRows";
import { ToolBurst } from "./ToolBurst";

let nextId = 1;
const at = (s: number) => `2026-10-19T05:00:${String(s).padStart(2, "0")}Z`;
function row(kind: string, content: string, sec: number): AgentMessage {
  return { id: nextId++, session_id: "s1", author: "hands", kind, content, created_at: at(sec) };
}
const use = (id: string, name: string, input: Record<string, unknown>, sec: number) =>
  row("tool_use", JSON.stringify({ input, name, tool_use_id: id }), sec);
const result = (id: string, content: string, sec: number, isError = false) =>
  row("tool_result", JSON.stringify({ content, is_error: isError, tool_use_id: id }), sec);

function burstOf(msgs: AgentMessage[]): BurstRow {
  const rows = chatRows(msgs);
  const b = rows[0];
  if (rows.length !== 1 || b.kind !== "burst") throw new Error("fixture is not one burst");
  return b;
}

function Harness({ burst, onViewFile }: { burst: BurstRow; onViewFile?: (p: string) => void }) {
  const [open, setOpen] = useState(false);
  return <ToolBurst burst={burst} expanded={open} onToggle={() => setOpen((v) => !v)} onViewFile={onViewFile} />;
}

const PNG = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";

describe("ToolBurst", () => {
  it("folds a run into one line: how many, which tools, how long, how many failed", () => {
    const burst = burstOf([
      use("a", "Bash", { command: "npx tsc --noEmit" }, 0),
      result("a", "Exit code 2\nerror TS2304", 3, true),
      use("b", "Read", { file_path: "/p/src/lib/x.ts" }, 4),
      result("b", "1\tconst x = 1;", 5),
      use("c", "Bash", { command: "npx vitest run", description: "Run the suite" }, 6),
      result("c", "Tests 734 passed", 14),
    ]);
    render(<Harness burst={burst} />);
    const line = screen.getByRole("button", { name: /3 tools/ });
    expect(line).toHaveAttribute("aria-expanded", "false");
    expect(line).toHaveTextContent("3 tools · Bash ×2, Read · 14s · 1 failed");
    expect(within(line).getByText("· 1 failed")).toHaveClass("text-error");
    // Folded: no call lines, no raw output on screen.
    expect(screen.queryByText(/Tests 734 passed/)).toBeNull();
  });

  it("opens to one human line per call with its result, and the raw call behind 'raw'", () => {
    const burst = burstOf([
      use("a", "Bash", { command: "git status", description: "Show working tree status" }, 0),
      result("a", "On branch main\nnothing to commit", 1),
      use("b", "Bash", { command: "false" }, 2),
      result("b", "Exit code 1", 3, true),
    ]);
    render(<Harness burst={burst} />);
    fireEvent.click(screen.getByRole("button", { name: /2 tools/ }));
    const items = screen.getAllByRole("listitem");
    expect(items).toHaveLength(2);
    expect(items[0]).toHaveTextContent("Bash · Show working tree status — On branch main");
    expect(within(items[0]).getByTitle("done")).toBeInTheDocument();
    expect(within(items[1]).getByTitle("failed")).toBeInTheDocument();
    expect(within(items[1]).getByText("Exit code 1")).toHaveClass("text-error");

    fireEvent.click(within(items[0]).getByRole("button", { name: "raw" }));
    const raw = items[0].querySelector("pre");
    expect(raw).not.toBeNull();
    expect(raw).toHaveTextContent('"command": "git status"');
    expect(raw).toHaveTextContent("nothing to commit");
  });

  it("shows a single call as the call itself, with its file link while folded", () => {
    const onViewFile = vi.fn();
    const burst = burstOf([
      use("a", "Read", { file_path: "/p/src/components/ChatPane.tsx" }, 0),
      result("a", "1\timport", 1),
    ]);
    render(<Harness burst={burst} onViewFile={onViewFile} />);
    expect(screen.getByRole("button", { name: /Read · components\/ChatPane.tsx/ })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "View ChatPane.tsx" }));
    expect(onViewFile).toHaveBeenCalledWith("/p/src/components/ChatPane.tsx");
  });

  it("offers the files a folded run wrote, not every path it read", () => {
    const onViewFile = vi.fn();
    const burst = burstOf([
      use("a", "Read", { file_path: "/p/src/a.ts" }, 0),
      result("a", "1\ta", 1),
      use("b", "Edit", { file_path: "/p/src/b.ts", old_string: "x", new_string: "y" }, 2),
      result("b", "The file has been updated.", 3),
      use("c", "Bash", { command: "cat --file /tmp/notes.md" }, 4),
      result("c", "notes", 5),
    ]);
    render(<Harness burst={burst} onViewFile={onViewFile} />);
    expect(screen.getAllByRole("button", { name: /^View / }).map((b) => b.textContent)).toEqual(["View b.ts"]);
    // Opened, every call that names a file has its own link.
    fireEvent.click(screen.getByRole("button", { name: /3 tools/ }));
    expect(screen.getAllByRole("button", { name: /^View / }).map((b) => b.textContent)).toEqual([
      "View a.ts",
      "View b.ts",
      "View notes.md",
    ]);
  });

  it("says a call is running, with its elapsed time, until its result lands", () => {
    const burst = burstOf([use("a", "Bash", { command: "cargo build --release" }, 0)]);
    render(<Harness burst={burst} />);
    const line = screen.getByRole("button", { name: /Bash/ });
    expect(line).toHaveTextContent(/running \d/);
    fireEvent.click(line);
    expect(screen.getByTitle("running")).toBeInTheDocument();
  });

  it("shows an image result as a thumbnail and never as base64 text", () => {
    const burst = burstOf([
      use("a", "mcp__chrome-devtools__take_screenshot", {}, 0),
      result(
        "a",
        JSON.stringify([
          { text: "Took a screenshot.", type: "text" },
          { source: { data: PNG, media_type: "image/png", type: "base64" }, type: "image" },
        ]),
        1,
      ),
    ]);
    const { container } = render(<Harness burst={burst} />);
    fireEvent.click(screen.getByRole("button", { name: /take_screenshot/ }));
    const img = screen.getByRole("img", { name: "Image 1 returned by take_screenshot" });
    expect(img).toHaveAttribute("src", `data:image/png;base64,${PNG}`);
    // A wide, short image must not push the chat column sideways (EYES).
    expect(img).toHaveClass("max-w-full", "max-h-40");
    fireEvent.click(screen.getByRole("button", { name: "raw" }));
    expect(container.textContent).not.toContain(PNG);
    expect(container.textContent).toContain("[1 image]");
  });
});
