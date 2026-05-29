import { describe, it, expect } from "vitest";
import { buildTree } from "./contextLibraryShared";
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
