// Group a classified unified `git diff` into per-file segments for the
// collapsible Apply-tab renderer. Mirrors the line classification produced by
// the Rust `parse_diff_lines` (tauri_cmd/docs.rs): each line carries a `kind`
// of "add" | "remove" | "hunk" | "file" | "context".

export interface DiffLine {
  kind: string;
  text: string;
}

export interface DiffFileGroup {
  /** Display name (the new `b/` path, falling back to the old `a/` path). */
  file: string;
  lines: DiffLine[];
  adds: number;
  removes: number;
}

/** Extract the displayed filename from a `diff --git a/<old> b/<new>` line. */
export function parseDiffFileName(diffGitLine: string): string {
  const b = diffGitLine.match(/ b\/(.+)$/);
  if (b) return b[1];
  const a = diffGitLine.match(/ a\/(.+?) b\//);
  if (a) return a[1];
  return diffGitLine.replace(/^diff --git a?\/?/, "").trim();
}

/**
 * Split classified diff lines into one group per file. A new group starts at
 * each `diff --git ` header; any lines preceding the first header (rare —
 * e.g. a bare note) collect into a single leading "(diff)" group. Add/remove
 * counts are tallied per group for the summary badge.
 */
export function groupDiffByFile(lines: DiffLine[]): DiffFileGroup[] {
  const groups: DiffFileGroup[] = [];
  let current: DiffFileGroup | null = null;
  for (const line of lines) {
    const isFileHeader =
      line.kind === "file" && line.text.startsWith("diff --git ");
    if (isFileHeader) {
      current = {
        file: parseDiffFileName(line.text),
        lines: [],
        adds: 0,
        removes: 0,
      };
      groups.push(current);
    } else if (!current) {
      current = { file: "(diff)", lines: [], adds: 0, removes: 0 };
      groups.push(current);
    }
    current.lines.push(line);
    if (line.kind === "add") current.adds++;
    else if (line.kind === "remove") current.removes++;
  }
  return groups;
}
