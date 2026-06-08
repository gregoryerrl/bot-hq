import { describe, expect, it } from "vitest";
import {
  groupDiffByFile,
  parseDiffFileName,
  type DiffLine,
} from "./diffGroups";

// Mirrors the kinds emitted by the Rust `parse_diff_lines` classifier.
const line = (kind: string, text: string): DiffLine => ({ kind, text });

describe("parseDiffFileName", () => {
  it("prefers the new (b/) path", () => {
    expect(parseDiffFileName("diff --git a/src/old.ts b/src/new.ts")).toBe(
      "src/new.ts",
    );
  });

  it("falls back to the a/ path when b/ is absent", () => {
    expect(parseDiffFileName("diff --git a/only.ts")).toBe("only.ts");
  });
});

describe("groupDiffByFile", () => {
  it("returns one group per diff --git header with tallied counts", () => {
    const lines = [
      line("file", "diff --git a/foo.ts b/foo.ts"),
      line("file", "index 000..111 100644"),
      line("hunk", "@@ -1,2 +1,2 @@"),
      line("context", " keep"),
      line("remove", "-gone"),
      line("add", "+new"),
      line("file", "diff --git a/bar.ts b/bar.ts"),
      line("hunk", "@@ -0,0 +1 @@"),
      line("add", "+added"),
    ];
    const groups = groupDiffByFile(lines);
    expect(groups).toHaveLength(2);
    expect(groups[0].file).toBe("foo.ts");
    expect(groups[0].adds).toBe(1);
    expect(groups[0].removes).toBe(1);
    expect(groups[0].lines).toHaveLength(6);
    expect(groups[1].file).toBe("bar.ts");
    expect(groups[1].adds).toBe(1);
    expect(groups[1].removes).toBe(0);
  });

  it("handles a rename header", () => {
    const groups = groupDiffByFile([
      line("file", "diff --git a/old.ts b/new.ts"),
      line("file", "similarity index 100%"),
      line("file", "rename from old.ts"),
      line("file", "rename to new.ts"),
    ]);
    expect(groups).toHaveLength(1);
    expect(groups[0].file).toBe("new.ts");
  });

  it("collects pre-header lines into a single leading group", () => {
    const groups = groupDiffByFile([
      line("context", "(session-start anchor lost)"),
      line("file", "diff --git a/foo.ts b/foo.ts"),
      line("add", "+x"),
    ]);
    expect(groups).toHaveLength(2);
    expect(groups[0].file).toBe("(diff)");
    expect(groups[1].file).toBe("foo.ts");
  });

  it("returns no groups for empty input", () => {
    expect(groupDiffByFile([])).toEqual([]);
  });
});
