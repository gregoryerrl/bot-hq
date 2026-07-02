import { open } from "@tauri-apps/plugin-dialog";
import { cn } from "../lib/cn";
import type { ClIndexEntryView } from "../lib/bindings";

// Shared types, constants, helpers, and icons for the Context Library view,
// split across ContextLibrary (shell), ContextLibrarySidebar, and
// ContextLibraryEditor.

// An open editor tab is either a file (content editor) or a folder (folder
// view). Discriminated on `kind` so the tab strip + editor area can route.
// (Proposals + measurement are no longer tabs here — they live on the
// Context Manager subtab.)
export type OpenTab =
  | { kind: "file"; project: string; filePath: string }
  | { kind: "folder"; project: string; folderPath: string };

// Stable identity for dedup, React keys, and active-tab matching.
export function tabKey(tab: OpenTab): string {
  if (tab.kind === "file") return `file:${tab.project}/${tab.filePath}`;
  return `folder:${tab.project}/${tab.folderPath}`;
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

// Right-click target in the tree. `path` is relative to the project CL root
// (file_path for files, folder_path for folders; "" = project root).
export interface CtxTarget {
  project: string;
  path: string;
  kind: "file" | "folder";
}

export const terminalInputClass = cn(
  "w-full border-0 border-b border-outline-variant bg-transparent",
  "rounded-none px-0 py-1 font-code-sm text-code-sm text-on-surface",
  "placeholder:text-on-surface-variant caret-primary",
  "focus:border-primary focus:outline-none",
);

/** Caps label above a form field. Shared by Settings + ModelsPanel. */
export function FieldLabel({ children }: { children: React.ReactNode }) {
  return (
    <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
      {children}
    </span>
  );
}

export function baseName(filePath: string): string {
  const parts = filePath.split("/");
  return parts[parts.length - 1] || filePath;
}

/**
 * Native OS folder picker (Finder / Explorer / file manager) via the Tauri
 * dialog plugin. Returns the chosen absolute path, or null if cancelled.
 * `title` labels the dialog; `defaultPath` pre-seeds it with the field's current
 * value. Single directory only.
 */
export async function pickFolder(
  title: string,
  defaultPath?: string,
): Promise<string | null> {
  const selected = await open({
    directory: true,
    multiple: false,
    title,
    defaultPath: defaultPath?.trim() || undefined,
  });
  return typeof selected === "string" ? selected : null;
}

// ============================================================================
// `_globals` categorization — SYSTEM vs GLOBAL
// ============================================================================

// Paths in the `_globals` bucket that bot-hq itself owns: agent custom
// instructions + custom-general-rules.md. Session spawn resolves these exact
// paths, so they are read+update only — no rename/delete/create around them.
// Deliberately a prefix predicate (not an exact-path set) so the `agents`
// folder itself, its folder-description rows, and any future agent files all
// classify as internal. Mirrored by `assert_not_protected_globals_path` in
// src/tauri_cmd/cl.rs — keep the two in sync.
export function isInternalGlobalsPath(path: string): boolean {
  return (
    path === "custom-general-rules.md" ||
    path === "agents" ||
    path.startsWith("agents/")
  );
}

export interface GlobalsSplit {
  system: { entries: ClIndexEntryView[]; folderPaths: string[] };
  global: { entries: ClIndexEntryView[]; folderPaths: string[] };
}

// Split the `_globals` bucket into the bot-hq-owned SYSTEM subtree and the
// loose cross-project GLOBAL subtree (scratch.md, tasks.md, user folders).
export function splitGlobals(
  entries: ClIndexEntryView[],
  folderPaths: string[],
): GlobalsSplit {
  return {
    system: {
      entries: entries.filter((e) => isInternalGlobalsPath(e.file_path)),
      folderPaths: folderPaths.filter(isInternalGlobalsPath),
    },
    global: {
      entries: entries.filter((e) => !isInternalGlobalsPath(e.file_path)),
      folderPaths: folderPaths.filter((p) => !isInternalGlobalsPath(p)),
    },
  };
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

// Tree-root project ids for the sidebar. Indexed projects (byProject keys)
// UNION registered projects — a freshly-registered project with no indexed
// files must still render, or Register appears to do nothing. During a text
// search only indexed (matching) projects show; a project FILTER pins the
// tree to that project even when it's empty. `_globals` renders via the
// SYSTEM/GLOBAL split, never as a Projects-category root, so it's excluded
// from the registered side (an indexed `_globals` key still passes through
// for the split's consumers).
export function treeProjectIds(
  indexed: string[],
  registered: string[],
  searchActive: boolean,
  projectFilter: string | null,
): string[] {
  const union = searchActive
    ? [...indexed]
    : [...new Set([...indexed, ...registered.filter((r) => r !== "_globals")])];
  return union
    .filter((id) => (projectFilter ? id === projectFilter : true))
    .sort();
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

export function ProposalsIcon({ className }: { className?: string }) {
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
      <path d="M5 4h14v5l-2 3H7L5 9V4z" />
      <path d="M7 12v7h10v-7" />
      <path d="M9 8h6" />
      <path d="M9 16h6" />
    </svg>
  );
}

export function MeasurementIcon({ className }: { className?: string }) {
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
      <path d="M4 20V4" />
      <path d="M4 20h16" />
      <rect x="7" y="12" width="3" height="5" />
      <rect x="12" y="8" width="3" height="9" />
      <rect x="17" y="14" width="3" height="3" />
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

// "Collapse all" — two chevrons meeting at the center (top points down, bottom
// points up), the VS-Code tree-toolbar fold glyph.
export function CollapseAllIcon({ className }: { className?: string }) {
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
      <polyline points="7 8 12 12 17 8" />
      <polyline points="7 16 12 12 17 16" />
    </svg>
  );
}
