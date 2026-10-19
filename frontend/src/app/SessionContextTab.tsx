import { useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useQueryClient } from "@tanstack/react-query";
import { useTauriQuery, errorMessage } from "../hooks/useInvoke";
import { cn } from "../lib/cn";
import type {
  ClFileContentView,
  ClFolderView,
  ClIndexEntryView,
  SessionProjectInfo,
} from "../lib/bindings";
import { buildTree, type TreeNode } from "./contextLibraryShared";
import { FileIcon, FolderIcon } from "../components/icons";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { Button } from "../components/ui/Button";
import { useDiscardGuard, useReportDirty } from "../hooks/useDiscardGuard";

// ============================================================================
// SessionContextTab — the session container's "Context" subtab: the Context
// Library scoped to THIS session's project — project tree + a lean read-write
// editor. Quick access without leaving the session room — the full
// cross-project surface stays in the Context Library page.
// ============================================================================

export function SessionContextTab({ sessionId }: { sessionId: string }) {
  const { data: info, isLoading } = useTauriQuery<SessionProjectInfo>(
    "get_session_project_info",
    { sessionId },
    { enabled: !!sessionId },
  );
  const project = info?.project ?? null;

  if (!project) {
    return (
      <div className="flex h-full items-center justify-center p-6 text-center">
        <p className="font-code-sm text-code-sm text-on-surface-variant">
          {isLoading
            ? "Resolving project…"
            : "No project is bound to this session — bind a working repo to browse its Context Library here."}
        </p>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <div className="flex flex-shrink-0 items-center border-b border-outline-variant px-4 py-1.5">
        <span className="ml-auto truncate font-code-sm text-code-sm text-on-surface-variant">
          {project}
        </span>
      </div>

      <ProjectFiles project={project} />
    </div>
  );
}

// ---------------------------------------------------------------------------
// Files: project tree (left) + lean editor (right)
// ---------------------------------------------------------------------------

function ProjectFiles({ project }: { project: string }) {
  const {
    data: entries = [],
    isLoading: entriesLoading,
    error: entriesError,
  } = useTauriQuery<ClIndexEntryView[]>("cl_index_search", { project, query: null });
  const { data: folders = [], isLoading: foldersLoading } = useTauriQuery<ClFolderView[]>(
    "cl_folder_search",
    { project, query: null },
  );
  const tree = useMemo(
    () =>
      buildTree(
        entries,
        folders.map((f) => f.folder_path).filter((p) => p !== ""),
      ),
    [entries, folders],
  );
  const [selected, setSelected] = useState<string | null>(null);
  // The open file's unsaved text: picking another file remounts the editor,
  // which used to drop the edit without a word.
  const [editorDirty, setEditorDirty] = useState(false);
  const discard = useDiscardGuard(editorDirty);
  const select = (path: string) => {
    if (path === selected) return;
    discard.guard(() => setSelected(path));
  };

  return (
    <div className="flex min-h-0 flex-1">
      <aside className="w-64 flex-shrink-0 overflow-y-auto overflow-x-hidden border-r border-outline-variant bg-surface-container py-1">
        {entries.length === 0 && folders.length === 0 ? (
          <p className="px-3 py-3 font-code-sm text-code-sm text-on-surface-variant">
            {entriesError
              ? `Couldn't read the index: ${entriesError.message}`
              : entriesLoading || foldersLoading
                ? "Loading…"
                : "No indexed files for this project yet."}
          </p>
        ) : (
          <FolderNode
            node={tree}
            depth={0}
            selected={selected}
            onSelect={select}
          />
        )}
      </aside>
      <div className="min-w-0 flex-1">
        {selected === null ? (
          <div className="flex h-full items-center justify-center p-6">
            <p className="font-code-sm text-code-sm text-on-surface-variant">
              Select a file to view or edit it.
            </p>
          </div>
        ) : (
          <LeanFileEditor
            key={selected}
            project={project}
            filePath={selected}
            onDirtyChange={setEditorDirty}
          />
        )}
      </div>
      <ConfirmDialog
        open={discard.asking}
        title="Discard changes?"
        message="This file has unsaved changes. Opening another file discards them."
        confirmLabel="Discard changes"
        confirmVariant="danger"
        onConfirm={discard.confirm}
        onCancel={discard.cancel}
      />
    </div>
  );
}

/** One folder level: files at this level + recursive subfolders. The root
 * node (path "") renders its children without a folder row. */
function FolderNode({
  node,
  depth,
  selected,
  onSelect,
}: {
  node: TreeNode;
  depth: number;
  selected: string | null;
  onSelect: (path: string) => void;
}) {
  const [collapsed, setCollapsed] = useState(false);
  const indent = { paddingLeft: `${depth * 12 + 12}px` };
  return (
    <>
      {node.path !== "" && (
        <button
          type="button"
          style={indent}
          onClick={() => setCollapsed((c) => !c)}
          className="flex w-full items-center gap-1.5 py-1 pr-2 text-left font-code-sm text-code-sm text-on-surface-variant transition-colors hover:text-on-surface"
        >
          <FolderIcon className="shrink-0" />
          <span className="truncate">{node.name}</span>
        </button>
      )}
      {(node.path === "" || !collapsed) && (
        <>
          {node.files.map((f) => (
            <button
              key={f.file_path}
              type="button"
              style={{
                paddingLeft: `${(node.path === "" ? 0 : depth + 1) * 12 + 12}px`,
              }}
              onClick={() => onSelect(f.file_path)}
              title={f.description}
              className={cn(
                "flex w-full items-center gap-1.5 py-1 pr-2 text-left font-code-sm text-code-sm transition-colors",
                selected === f.file_path
                  ? "bg-surface-container-highest text-on-surface"
                  : "text-on-surface-variant hover:bg-surface-container-high hover:text-on-surface",
              )}
            >
              <FileIcon className="shrink-0" />
              <span className="truncate">
                {f.file_path.split("/").pop()}
              </span>
            </button>
          ))}
          {node.folders.map((f) => (
            <FolderNode
              key={f.path}
              node={f}
              depth={node.path === "" ? depth : depth + 1}
              selected={selected}
              onSelect={onSelect}
            />
          ))}
        </>
      )}
    </>
  );
}

/** Minimal read-write editor on cl_read_file / cl_write_file. Deliberately
 * NOT the full-page EditorArea (that one is coupled to the library's
 * multi-tab state machine); same lossy-save guard: truncated or binary
 * reads are view-only. Keyed by filePath, so draft state resets per file. */
function LeanFileEditor({
  project,
  filePath,
  onDirtyChange,
}: {
  project: string;
  filePath: string;
  /** Unsaved text, for the file list's "Discard changes?" guard. */
  onDirtyChange?: (dirty: boolean) => void;
}) {
  const queryClient = useQueryClient();
  const { data: file, error: readError } = useTauriQuery<ClFileContentView>(
    "cl_read_file",
    { project, filePath },
  );
  const [draft, setDraft] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  const readOnly = !!file && (file.truncated || file.binary);
  const text = draft ?? file?.content ?? "";
  const dirty = !!file && draft !== null && draft !== file.content;
  useReportDirty(dirty, onDirtyChange);

  const onSave = async () => {
    if (!file || readOnly || !dirty || saving) return;
    setSaving(true);
    setSaveError(null);
    try {
      await invoke("cl_write_file", { project, filePath, content: text });
      queryClient.invalidateQueries({ queryKey: ["cl_read_file"] });
      queryClient.invalidateQueries({ queryKey: ["cl_index_search"] });
      setDraft(null);
    } catch (e) {
      setSaveError(errorMessage(e));
    } finally {
      setSaving(false);
    }
  };

  if (readError) {
    return (
      <div className="p-4 font-code-sm text-code-sm text-on-error-container">
        Failed to read {filePath}: {readError.message}
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <div className="flex flex-shrink-0 items-center justify-between gap-2 border-b border-outline-variant px-4 py-2">
        <p className="min-w-0 truncate font-code-sm text-code-sm text-on-surface">
          {filePath}
          {file?.truncated && (
            <span className="ml-2 text-warning">truncated to 1 MB</span>
          )}
          {file?.binary && (
            <span className="ml-2 text-warning">binary / non-UTF-8</span>
          )}
        </p>
        {readOnly ? (
          <span
            className="shrink-0 font-code-sm text-code-sm text-on-surface-variant"
            title="Read-only: editing a truncated or non-UTF-8 file could corrupt it"
          >
            read-only
          </span>
        ) : (
          <Button
            variant={dirty ? "primary" : "secondary"}
            size="sm"
            disabled={!dirty || saving}
            onClick={onSave}
            className="shrink-0"
          >
            {saving ? "Saving…" : "Save"}
          </Button>
        )}
      </div>
      {saveError && (
        <div className="border-b border-outline-variant bg-error-container/30 px-4 py-2 font-code-sm text-code-sm text-on-error-container">
          Save failed: {saveError}
        </div>
      )}
      <textarea
        value={text}
        readOnly={readOnly}
        onChange={(e) => setDraft(e.target.value)}
        spellCheck={false}
        aria-label={`Edit ${filePath}`}
        className="min-h-0 w-full flex-1 resize-none bg-surface p-4 font-code-sm text-code-sm text-on-surface focus:outline-none"
      />
    </div>
  );
}
