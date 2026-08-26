import { describe, it, expect } from "vitest";
import { lineDiff } from "./lineDiff";

describe("lineDiff", () => {
  it("identical inputs are all same-lines", () => {
    expect(lineDiff("a\nb", "a\nb")).toEqual([
      { kind: "same", text: "a" },
      { kind: "same", text: "b" },
    ]);
  });

  it("del comes from `from`, add from `to`", () => {
    // The Roles-tab orientation: from = shipped default, to = the user's
    // prose — so del is what the user removed, add is what they wrote.
    const d = lineDiff("keep\ndefault-only", "keep\nuser-only");
    expect(d).toEqual([
      { kind: "same", text: "keep" },
      { kind: "del", text: "default-only" },
      { kind: "add", text: "user-only" },
    ]);
  });

  it("pure insertion and pure deletion both terminate", () => {
    // "" splits to one empty line the other side doesn't have; the walk
    // prefers del on ties, so it leads.
    expect(lineDiff("", "x")).toEqual([
      { kind: "del", text: "" },
      { kind: "add", text: "x" },
    ]);
    expect(lineDiff("x\ny\nz", "y")).toEqual([
      { kind: "del", text: "x" },
      { kind: "same", text: "y" },
      { kind: "del", text: "z" },
    ]);
  });

  it("every input line appears exactly once in the output", () => {
    const from = "a\nb\nc\nd";
    const to = "a\nc\nx\nd";
    const d = lineDiff(from, to);
    expect(d.filter((l) => l.kind !== "add").map((l) => l.text)).toEqual(
      from.split("\n"),
    );
    expect(d.filter((l) => l.kind !== "del").map((l) => l.text)).toEqual(
      to.split("\n"),
    );
  });
});
