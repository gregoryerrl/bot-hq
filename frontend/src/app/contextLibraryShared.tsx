import { open } from "@tauri-apps/plugin-dialog";
import { cn } from "../lib/cn";
import type { ClIndexEntryView } from "../lib/bindings";

// Shared types, constants, helpers, and icons for the Context Library view,
// split across ContextLibrary (shell), ContextLibrarySidebar, and
// ContextLibraryEditor.

// An open editor tab is either a file (content editor) or a folder (folder
// view). Discriminated on `kind` so the tab strip + editor area can route.
// (Measurement is not a tab here — it lives on the Context Manager subtab.)
export type OpenTab =
  | { kind: "file"; project: string; filePath: string }
  | { kind: "folder"; project: string; folderPath: string };

// Stable identity for dedup, React keys, and active-tab matching.
export function tabKey(tab: OpenTab): string {
  if (tab.kind === "file") return `file:${tab.project}/${tab.filePath}`;
  return `folder:${tab.project}/${tab.folderPath}`;
}

/** What the open tabs become when a project is deleted or RENAMED.
 *
 * Delete (`replacement` undefined): drop them. The files are gone, so a tab
 * pointing at one has nothing behind it and nothing to save back to.
 *
 * Rename: **retarget, do not drop.** The old code filtered by the old project
 * name in both cases, so renaming a project closed every tab you had open in it
 * — including one holding unsaved text — while the file and the text both still
 * existed under the new name. The rename has already happened by the time this
 * runs (it is a notification, not a request), so a confirm would have nothing to
 * offer; following the file is the only answer that keeps anything.
 *
 * Known residual: `tabKey` includes the project, so a retargeted tab changes
 * identity and its pane remounts — the tab and the file survive, the unsaved
 * text does not. Fixing that needs tabs to carry a stable id instead of being
 * keyed by their contents; indexed rather than done here.
 *
 * **Depends on an upstream guarantee, named here because it is invisible from
 * this file:** the retarget is a `map`, so it would produce two tabs sharing a
 * `tabKey` if a project could be renamed ONTO an existing project's name — a
 * duplicate React key in the strip and in the pane list. It cannot:
 * `cl_rename_project` rejects that up front (`src/tauri_cmd/cl.rs:733-737`,
 * "a project named '{new_name}' already exists"). If that check is ever
 * relaxed, dedupe here.
 */
export function tabsAfterProjectGone(
  tabs: OpenTab[],
  name: string,
  replacement?: string,
): OpenTab[] {
  if (!replacement) return tabs.filter((t) => t.project !== name);
  return tabs.map((t) => (t.project === name ? { ...t, project: replacement } : t));
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

// Paths in the `_globals` bucket that bot-hq itself owns:
// custom-instructions.md (one file, appended to every agent's prompt) +
// custom-general-rules.md. Session spawn resolves these exact paths, so they
// are read+update only — no rename/delete/create around them. The `agents/`
// prefix is the legacy pre-consolidation location — still classified internal
// so stragglers on partially-migrated installs stay protected. Mirrored by
// `assert_not_protected_globals_path` in src/tauri_cmd/cl.rs — keep the two
// in sync.
export function isInternalGlobalsPath(path: string): boolean {
  return (
    path === "custom-general-rules.md" ||
    path === "custom-instructions.md" ||
    path === "agents" ||
    path.startsWith("agents/")
  );
}

interface GlobalsSplit {
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
