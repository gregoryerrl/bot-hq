/**
 * Clipboard / drag-drop path helpers for the composer (ideas.md 2026-08-24).
 *
 * A file copied in Finder pastes as `text/uri-list` (`file:///…`, one per
 * line, `#` lines are comments per RFC 2483); a file dragged onto the window
 * arrives through Tauri's onDragDropEvent with real OS paths. Both end as
 * absolute paths inserted into the box — the agent Reads them verbatim.
 */

/** The `file://` paths in a `text/uri-list` payload, decoded. Non-file URIs
 *  (https:, data:) are ignored — pasting a browser link is prose, not a file. */
export function uriListToPaths(uriList: string): string[] {
  return uriList
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l !== "" && !l.startsWith("#") && l.startsWith("file://"))
    .map((l) => {
      try {
        return decodeURIComponent(l.replace(/^file:\/\//, ""));
      } catch {
        return l.replace(/^file:\/\//, "");
      }
    });
}

// (`quotePath`/`pathsToInsertText` were removed in round 13: dropped files
// now insert short `#name.ext` tokens and the composer's expansion map wraps
// the real path in inline code at Send — no shell-quoting of raw paths in the
// box any more.)
