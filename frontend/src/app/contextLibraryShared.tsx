import { cn } from "../lib/cn";
import type { ClIndexEntryView } from "../lib/bindings";

// Shared types, constants, helpers, and icons for the Context Library view,
// split across ContextLibrary (shell), ContextLibrarySidebar, and
// ContextLibraryEditor.

// An open editor tab is either a file (content editor) or a folder (folder
// view). Discriminated on `kind` so the tab strip + editor area can route.
export type OpenTab =
  | { kind: "file"; project: string; filePath: string }
  | { kind: "folder"; project: string; folderPath: string };

// Stable identity for dedup, React keys, and active-tab matching.
export function tabKey(tab: OpenTab): string {
  return tab.kind === "file"
    ? `file:${tab.project}/${tab.filePath}`
    : `folder:${tab.project}/${tab.folderPath}`;
}

// Tab strip label. A folder with an empty path is the project root, so it
// shows the project name; everything else shows the trailing path segment.
export function tabLabel(tab: OpenTab): string {
  const path = tab.kind === "file" ? tab.filePath : tab.folderPath;
  return path === "" ? tab.project : baseName(path);
}

// Collapse-state key for a tree node. The project-root node uses folderPath "".
export function collapseKey(project: string, folderPath: string): string {
  return `${project}\t${folderPath}`;
}

export const terminalInputClass = cn(
  "w-full border-0 border-b border-outline-variant bg-transparent",
  "rounded-none px-0 py-1 font-code-sm text-code-sm text-on-surface",
  "placeholder:text-on-surface-variant caret-primary",
  "focus:border-primary focus:outline-none",
);

export function baseName(filePath: string): string {
  const parts = filePath.split("/");
  return parts[parts.length - 1] || filePath;
}

// ============================================================================
// Folder tree
// ============================================================================

export interface TreeNode {
  /** Trailing path segment; "" for the project root. */
  name: string;
  /** Full folder path relative to the project CL root; "" = root. */
  path: string;
  folders: TreeNode[];
  /** Files directly in this folder. */
  files: ClIndexEntryView[];
}

// Build a nested folder tree for one project from its flat index entries
// (file_path may contain "/"), plus any folder paths that carry a description
// but hold no files (so a described-but-empty folder still appears). Folders
// and files are sorted alphabetically at every level.
export function buildTree(
  entries: ClIndexEntryView[],
  folderPaths: string[] = [],
): TreeNode {
  const root: TreeNode = { name: "", path: "", folders: [], files: [] };

  const ensureFolder = (segments: string[]): TreeNode => {
    let node = root;
    let acc = "";
    for (const seg of segments) {
      acc = acc ? `${acc}/${seg}` : seg;
      let child = node.folders.find((f) => f.name === seg);
      if (!child) {
        child = { name: seg, path: acc, folders: [], files: [] };
        node.folders.push(child);
      }
      node = child;
    }
    return node;
  };

  for (const e of entries) {
    const segs = e.file_path.split("/");
    segs.pop(); // last segment is the file name; the rest are folders
    ensureFolder(segs).files.push(e);
  }
  for (const fp of folderPaths) {
    if (fp) ensureFolder(fp.split("/"));
  }

  const sortNode = (n: TreeNode) => {
    n.folders.sort((a, b) => a.name.localeCompare(b.name));
    n.files.sort((a, b) => a.file_path.localeCompare(b.file_path));
    n.folders.forEach(sortNode);
  };
  sortNode(root);
  return root;
}

// ============================================================================
// Inline SVG icons
// ============================================================================

export function PlusIcon({ className }: { className?: string }) {
  return (
    <svg
      className={cn("size-4", className)}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <line x1="12" y1="5" x2="12" y2="19" />
      <line x1="5" y1="12" x2="19" y2="12" />
    </svg>
  );
}

export function RefreshIcon({ className }: { className?: string }) {
  return (
    <svg
      className={cn("size-3.5", className)}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <polyline points="23 4 23 10 17 10" />
      <path d="M20.49 15a9 9 0 11-2.12-9.36L23 10" />
    </svg>
  );
}

export function FileIcon({ className }: { className?: string }) {
  return (
    <svg
      className={cn("size-3.5", className)}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" />
      <polyline points="14 2 14 8 20 8" />
    </svg>
  );
}

export function FolderIcon({ className }: { className?: string }) {
  return (
    <svg
      className={cn("size-3.5", className)}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <path d="M3 7a2 2 0 012-2h4l2 2h8a2 2 0 012 2v8a2 2 0 01-2 2H5a2 2 0 01-2-2z" />
    </svg>
  );
}

export function CloseIcon({ className }: { className?: string }) {
  return (
    <svg
      className={cn("size-3.5", className)}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <line x1="18" y1="6" x2="6" y2="18" />
      <line x1="6" y1="6" x2="18" y2="18" />
    </svg>
  );
}

export function SaveIcon({ className }: { className?: string }) {
  return (
    <svg
      className={cn("size-3.5", className)}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <path d="M19 21H5a2 2 0 01-2-2V5a2 2 0 012-2h11l5 5v11a2 2 0 01-2 2z" />
      <polyline points="17 21 17 13 7 13 7 21" />
      <polyline points="7 3 7 8 15 8" />
    </svg>
  );
}
