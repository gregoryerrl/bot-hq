import { describe, expect, it } from "vitest";
import { diffLines } from "./proposalDiff";

describe("diffLines", () => {
  it("marks identical content as all context", () => {
    const lines = diffLines("a\nb\nc", "a\nb\nc");
    expect(lines).toEqual([
      { kind: "context", text: "a" },
      { kind: "context", text: "b" },
      { kind: "context", text: "c" },
    ]);
  });

  it("detects a pure insertion", () => {
    const lines = diffLines("a\nc", "a\nb\nc");
    expect(lines).toEqual([
      { kind: "context", text: "a" },
      { kind: "add", text: "b" },
      { kind: "context", text: "c" },
    ]);
  });

  it("detects a pure deletion", () => {
    const lines = diffLines("a\nb\nc", "a\nc");
    expect(lines).toEqual([
      { kind: "context", text: "a" },
      { kind: "remove", text: "b" },
      { kind: "context", text: "c" },
    ]);
  });

  it("renders a changed line as remove + add", () => {
    const lines = diffLines("a\nold\nc", "a\nnew\nc");
    expect(lines.filter((l) => l.kind === "remove")).toEqual([
      { kind: "remove", text: "old" },
    ]);
    expect(lines.filter((l) => l.kind === "add")).toEqual([
      { kind: "add", text: "new" },
    ]);
    expect(lines[0]).toEqual({ kind: "context", text: "a" });
    expect(lines[3]).toEqual({ kind: "context", text: "c" });
  });

  it("treats an empty current file as all additions", () => {
    // "" splits to [""] — the empty line pairs off as remove, rest are adds.
    const lines = diffLines("", "a\nb");
    expect(lines.filter((l) => l.kind === "add").map((l) => l.text)).toEqual([
      "a",
      "b",
    ]);
    expect(lines.some((l) => l.kind === "context")).toBe(false);
  });

  it("keeps unchanged prefix/suffix as context around a rewrite", () => {
    const current = "# title\nkeep me\nstale fact one\nstale fact two\nfooter";
    const proposed = "# title\nkeep me\nfresh fact\nfooter";
    const lines = diffLines(current, proposed);
    expect(lines[0]).toEqual({ kind: "context", text: "# title" });
    expect(lines[1]).toEqual({ kind: "context", text: "keep me" });
    expect(lines[lines.length - 1]).toEqual({ kind: "context", text: "footer" });
    expect(lines.filter((l) => l.kind === "remove").length).toBe(2);
    expect(lines.filter((l) => l.kind === "add").length).toBe(1);
  });
});
