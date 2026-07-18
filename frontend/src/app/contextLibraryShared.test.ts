import { describe, it, expect } from "vitest";
import {
  buildTree,
  isInternalGlobalsPath,
  splitGlobals,
  treeProjectIds,
} from "./contextLibraryShared";
import type { ClIndexEntryView } from "../lib/bindings";

const entry = (id: number, file_path: string): ClIndexEntryView => ({
  id,
  project_id: "p",
  file_path,
  description: "",
  tags: null,
  created_at: "",
  updated_at: "",
});

describe("buildTree", () => {
  it("nests files into folders by path segments", () => {
    const root = buildTree([
      entry(1, "readme.md"),
      entry(2, "archive/a.md"),
      entry(3, "archive/b.md"),
      entry(4, "archive/old/c.md"),
      entry(5, "plans/p.md"),
    ]);

    // root holds the top-level file + the two folders (sorted)
    expect(root.files.map((f) => f.file_path)).toEqual(["readme.md"]);
    expect(root.folders.map((f) => f.name)).toEqual(["archive", "plans"]);

    const archive = root.folders.find((f) => f.name === "archive")!;
    expect(archive.path).toBe("archive");
    expect(archive.files.map((f) => f.file_path)).toEqual([
      "archive/a.md",
      "archive/b.md",
    ]);
    expect(archive.folders.map((f) => f.name)).toEqual(["old"]);

    const old = archive.folders[0];
    expect(old.path).toBe("archive/old");
    expect(old.files.map((f) => f.file_path)).toEqual(["archive/old/c.md"]);
  });

  it("includes described-but-empty folders from folderPaths", () => {
    const root = buildTree([entry(1, "a.md")], ["notes", "notes/sub"]);
    const notes = root.folders.find((f) => f.name === "notes")!;
    expect(notes).toBeTruthy();
    expect(notes.path).toBe("notes");
    expect(notes.folders.map((f) => f.name)).toEqual(["sub"]);
    expect(notes.files).toEqual([]);
  });

  it("sorts files alphabetically within a folder", () => {
    const root = buildTree([entry(2, "b.md"), entry(1, "a.md")]);
    expect(root.files.map((f) => f.file_path)).toEqual(["a.md", "b.md"]);
  });
});

describe("isInternalGlobalsPath", () => {
  it("classifies bot-hq-owned paths as internal", () => {
    expect(isInternalGlobalsPath("custom-general-rules.md")).toBe(true);
    expect(isInternalGlobalsPath("custom-instructions.md")).toBe(true);
    // Legacy pre-consolidation locations stay internal so stragglers on
    // partially-migrated installs remain protected.
    expect(isInternalGlobalsPath("agents")).toBe(true);
    expect(isInternalGlobalsPath("agents/brian/custom-instruction.md")).toBe(
      true,
    );
  });

  it("leaves loose cross-project paths external", () => {
    expect(isInternalGlobalsPath("scratch.md")).toBe(false);
    expect(isInternalGlobalsPath("tasks.md")).toBe(false);
    expect(isInternalGlobalsPath("notes/agents.md")).toBe(false);
    // prefix must be a path segment, not a substring
    expect(isInternalGlobalsPath("agents-archive/old.md")).toBe(false);
    expect(isInternalGlobalsPath("")).toBe(false);
  });
});

describe("splitGlobals", () => {
  it("routes internal entries+folders to system, the rest to global", () => {
    const entries = [
      entry(1, "custom-general-rules.md"),
      entry(2, "agents/brian/custom-instruction.md"),
      entry(3, "scratch.md"),
      entry(4, "notes/draft.md"),
    ];
    const folders = ["agents", "agents/brian", "notes"];
    const split = splitGlobals(entries, folders);

    expect(split.system.entries.map((e) => e.file_path)).toEqual([
      "custom-general-rules.md",
      "agents/brian/custom-instruction.md",
    ]);
    expect(split.system.folderPaths).toEqual(["agents", "agents/brian"]);
    expect(split.global.entries.map((e) => e.file_path)).toEqual([
      "scratch.md",
      "notes/draft.md",
    ]);
    expect(split.global.folderPaths).toEqual(["notes"]);
  });

  it("handles an empty bucket", () => {
    const split = splitGlobals([], []);
    expect(split.system.entries).toEqual([]);
    expect(split.global.entries).toEqual([]);
  });
});

describe("treeProjectIds", () => {
  it("unions registered projects so an unindexed one still renders", () => {
    const ids = treeProjectIds(["bot-hq"], ["bot-hq", "fresh"], false, null);
    expect(ids).toEqual(["bot-hq", "fresh"]);
  });

  it("excludes _globals from the registered side but keeps it when indexed", () => {
    expect(treeProjectIds([], ["_globals", "a"], false, null)).toEqual(["a"]);
    expect(treeProjectIds(["_globals"], ["a"], false, null)).toEqual([
      "_globals",
      "a",
    ]);
  });

  it("shows only indexed matches during a text search", () => {
    const ids = treeProjectIds(["hit"], ["hit", "miss"], true, null);
    expect(ids).toEqual(["hit"]);
  });

  it("pins to the project filter even when it has no indexed files", () => {
    const ids = treeProjectIds(["other"], ["other", "empty"], false, "empty");
    expect(ids).toEqual(["empty"]);
  });

  it("sorts and dedups", () => {
    const ids = treeProjectIds(["b", "a"], ["a", "c"], false, null);
    expect(ids).toEqual(["a", "b", "c"]);
  });
});
