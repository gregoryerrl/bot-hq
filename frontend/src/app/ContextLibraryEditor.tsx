import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTauriQuery, useTauriMutation, errorMessage } from "../hooks/useInvoke";
import { cn } from "../lib/cn";
import type {
  ClFileContentView,
  ClFolderView,
  ClIndexEntryView,
  Policy,
  ProjectView,
} from "../lib/bindings";
import {
  CloseIcon,
  FileIcon,
  FolderIcon,
  type OpenTab,
  SaveIcon,
  tabKey,
  tabLabel,
  terminalInputClass,
} from "./contextLibraryShared";
import { FolderView } from "./ContextLibraryFolderView";
import { PolicyForm } from "../components/PolicyForm";

/** A CL file is a project policy blueprint when its basename is `policy.yaml`. */
function isPolicyFile(filePath: string): boolean {
  return filePath.split("/").pop() === "policy.yaml";
}

// ============================================================================
// EditorArea — tab strip + active tab content
// ============================================================================

interface EditorAreaProps {
  tabs: OpenTab[];
  activeTabIndex: number;
  onSelectTab: (i: number) => void;
  onCloseTab: (i: number) => void;
  activeTab: OpenTab | null;
  entries: ClIndexEntryView[];
  folders: ClFolderView[];
  projects: ProjectView[];
  onRefetchIndex: () => void;
  onRefetchFolders: () => void;
  onProjectChanged: () => void;
}

export function EditorArea({
  tabs,
  activeTabIndex,
  onSelectTab,
  onCloseTab,
  activeTab,
  entries,
  folders,
  projects,
  onRefetchIndex,
  onRefetchFolders,
  onProjectChanged,
}: EditorAreaProps) {
  return (
    <div className="flex min-w-0 flex-1 flex-col">
      {tabs.length > 0 && (
        <TabStrip
          tabs={tabs}
          activeTabIndex={activeTabIndex}
          onSelectTab={onSelectTab}
          onCloseTab={onCloseTab}
        />
      )}
      {activeTab == null ? (
        <EmptyEditor />
      ) : activeTab.kind === "folder" ? (
        <FolderView
          key={tabKey(activeTab)}
          tab={activeTab}
          folders={folders}
          project={
            projects.find((p) => p.name === activeTab.project) ?? null
          }
          onSaved={onRefetchFolders}
          onProjectChanged={onProjectChanged}
        />
      ) : isPolicyFile(activeTab.filePath) ? (
        <ProjectPolicyEditor
          key={tabKey(activeTab)}
          tab={activeTab}
          entries={entries}
          onRefetchIndex={onRefetchIndex}
        />
      ) : (
        <EditorPane
          key={tabKey(activeTab)}
          tab={activeTab}
          entries={entries}
          onRefetchIndex={onRefetchIndex}
        />
      )}
    </div>
  );
}

// ============================================================================
// ProjectPolicyEditor — structured project-policy editor for `policy.yaml`,
// with a Raw YAML escape hatch back to the plain text editor. The structured
// form (default) always writes valid YAML via set_project_policy; the raw view
// is for hand-editing comments / fields the form doesn't model.
// ============================================================================

function ProjectPolicyEditor({
  tab,
  entries,
  onRefetchIndex,
}: {
  tab: Extract<OpenTab, { kind: "file" }>;
  entries: ClIndexEntryView[];
  onRefetchIndex: () => void;
}) {
  const [view, setView] = useState<"form" | "raw">("form");
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex flex-shrink-0 items-center justify-between gap-3 border-b border-outline-variant bg-surface-container-low px-4 py-2">
        <div className="min-w-0">
          <p className="truncate font-code-sm text-code-sm text-on-surface">
            {tab.filePath}
          </p>
          <p className="font-code-sm text-code-sm text-on-surface-variant">
            {tab.project} · project policy
          </p>
        </div>
        <div className="flex shrink-0 overflow-hidden rounded border border-outline-variant">
          {(["form", "raw"] as const).map((v) => (
            <button
              key={v}
              type="button"
              onClick={() => setView(v)}
              className={cn(
                "px-3 py-1 font-label-caps text-label-caps transition-colors",
                view === v
                  ? "bg-primary/15 text-primary"
                  : "bg-transparent text-on-surface-variant hover:text-on-surface",
              )}
            >
              {v === "form" ? "Form" : "Raw YAML"}
            </button>
          ))}
        </div>
      </div>
      {view === "form" ? (
        <ProjectPolicyForm project={tab.project} />
      ) : (
        <EditorPane
          tab={tab}
          entries={entries}
          onRefetchIndex={onRefetchIndex}
        />
      )}
    </div>
  );
}

function ProjectPolicyForm({ project }: { project: string }) {
  const { data: server, refetch, isLoading } = useTauriQuery<Policy>(
    "get_project_policy",
    { project },
  );
  const save = useTauriMutation<void, { project: string; policy: Policy }>(
    "set_project_policy",
  );

  const serverJson = JSON.stringify(server ?? {});
  const [draft, setDraft] = useState<Policy>(server ?? {});
  const lastServer = useRef(serverJson);
  useEffect(() => {
    if (lastServer.current !== serverJson) {
      lastServer.current = serverJson;
      setDraft(server ?? {});
    }
  }, [serverJson, server]);

  const dirty = JSON.stringify(draft) !== serverJson;

  const onSave = async () => {
    await save.mutateAsync({ project, policy: draft });
    await refetch();
  };

  return (
    <div className="min-h-0 flex-1 overflow-auto px-5 py-5">
      <div className="mb-4 flex items-start justify-between gap-4">
        <p className="max-w-prose font-body-md text-body-md text-on-surface-variant">
          Per-project overrides on top of the global policy. Empty fields
          inherit the global tier; non-empty lists / non-default toggles
          replace it. The form rewrites <code>policy.yaml</code> as clean YAML
          on save — switch to Raw YAML to preserve comments or hand-edit.
        </p>
        {dirty && (
          <button
            type="button"
            onClick={onSave}
            disabled={save.isPending}
            className="inline-flex shrink-0 items-center gap-2 rounded border border-primary bg-primary px-3 py-1.5 font-code-sm text-code-sm text-on-primary transition-colors hover:bg-primary-fixed disabled:opacity-50"
          >
            <SaveIcon />
            {save.isPending ? "Saving…" : "Save policy"}
          </button>
        )}
      </div>
      {isLoading ? (
        <div className="h-48 animate-pulse rounded-lg border border-outline-variant bg-surface-container" />
      ) : (
        <PolicyForm value={draft} onChange={setDraft} disabled={save.isPending} />
      )}
      {save.error && (
        <p className="mt-4 rounded border border-error/40 bg-error-container/20 px-3 py-2 font-code-sm text-code-sm text-on-error-container">
          Save failed: {save.error.message}
        </p>
      )}
    </div>
  );
}

// ============================================================================
// TabStrip — horizontal tab bar at the top of the editor area
// ============================================================================

function TabStrip({
  tabs,
  activeTabIndex,
  onSelectTab,
  onCloseTab,
}: {
  tabs: OpenTab[];
  activeTabIndex: number;
  onSelectTab: (i: number) => void;
  onCloseTab: (i: number) => void;
}) {
  return (
    <div className="flex flex-shrink-0 items-center overflow-x-auto border-b border-outline-variant bg-surface-container">
      {tabs.map((t, i) => {
        const active = i === activeTabIndex;
        const path = t.kind === "file" ? t.filePath : t.folderPath;
        return (
          <div
            key={tabKey(t)}
            className={cn(
              "group flex shrink-0 items-center gap-2 border-r border-outline-variant/40 px-3 py-2",
              "font-code-sm text-code-sm transition-colors",
              active
                ? "bg-background text-on-surface"
                : "text-on-surface-variant hover:bg-surface-container-high hover:text-on-surface",
            )}
          >
            <button
              type="button"
              onClick={() => onSelectTab(i)}
              title={`${t.project} — ${path || "(root)"}`}
              className="flex max-w-[200px] items-center gap-1.5"
            >
              {t.kind === "folder" ? (
                <FolderIcon className="shrink-0 text-on-surface-variant/60" />
              ) : (
                <FileIcon className="shrink-0 text-on-surface-variant/60" />
              )}
              <span className="truncate">{tabLabel(t)}</span>
            </button>
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                onCloseTab(i);
              }}
              aria-label={`Close ${path || t.project}`}
              className="rounded p-0.5 text-on-surface-variant/60 transition-colors hover:bg-surface-container-highest hover:text-on-surface"
            >
              <CloseIcon />
            </button>
          </div>
        );
      })}
    </div>
  );
}

// ============================================================================
// EditorPane — file content + line-number gutter + description + footer
// ============================================================================

function EditorPane({
  tab,
  entries,
  onRefetchIndex,
}: {
  tab: Extract<OpenTab, { kind: "file" }>;
  entries: ClIndexEntryView[];
  onRefetchIndex: () => void;
}) {
  const {
    data: fileContent,
    isFetching,
    error: fileError,
    refetch,
  } = useTauriQuery<ClFileContentView>("cl_read_file", {
    project: tab.project,
    filePath: tab.filePath,
  });

  // Editable working copy. EditorArea keys EditorPane by path, so this pane
  // remounts per file — a one-shot seed when content first arrives is enough
  // (there's no cross-file stale draft to clear).
  const [draft, setDraft] = useState<string | null>(null);
  if (draft === null && fileContent) setDraft(fileContent.content);

  // Refuse edits that would lose data on save: a truncated read (we only hold
  // the first 1 MB) or a binary / non-UTF-8 file (content is a lossy decode,
  // so writing it back would corrupt the original bytes).
  const readOnly =
    !!fileContent && (fileContent.truncated || fileContent.binary);
  const dirty =
    !readOnly &&
    fileContent != null &&
    draft != null &&
    draft !== fileContent.content;

  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  // Metadata (CL index entry) editor is collapsed by default — it's secondary
  // to the file content. The toggle lives in the header; `metadataDirty` lets
  // us badge the toggle when there are unsaved metadata edits while collapsed.
  const [showMetadata, setShowMetadata] = useState(false);
  const [metadataDirty, setMetadataDirty] = useState(false);

  const handleSave = async () => {
    if (!dirty || saving || draft == null) return;
    setSaving(true);
    setSaveError(null);
    try {
      await invoke("cl_write_file", {
        project: tab.project,
        filePath: tab.filePath,
        content: draft,
      });
      await refetch(); // resync the baseline so `dirty` clears
    } catch (e) {
      setSaveError(errorMessage(e));
    } finally {
      setSaving(false);
    }
  };

  const entry = useMemo(
    () =>
      entries.find(
        (e) => e.project_id === tab.project && e.file_path === tab.filePath,
      ) ?? null,
    [entries, tab.project, tab.filePath],
  );

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header className="flex flex-shrink-0 items-center justify-between gap-3 border-b border-outline-variant bg-surface-container-low px-4 py-2">
        <div className="min-w-0">
          <p className="truncate font-code-sm text-code-sm text-on-surface">
            {tab.filePath}
          </p>
          <p className="font-code-sm text-code-sm text-on-surface-variant">
            {tab.project}
            {fileContent && (
              <>
                <span className="mx-2 text-on-surface-variant/60">·</span>
                {fileContent.size_bytes.toLocaleString()} bytes
                {fileContent.truncated && (
                  <>
                    <span className="mx-2 text-on-surface-variant/60">·</span>
                    <span className="text-amber-400">
                      truncated to 1 MB
                    </span>
                  </>
                )}
                {fileContent.binary && (
                  <>
                    <span className="mx-2 text-on-surface-variant/60">·</span>
                    <span className="text-amber-400">binary / non-UTF-8</span>
                  </>
                )}
              </>
            )}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {readOnly && (
            <span
              className="rounded border border-outline-variant bg-surface-container-high px-2 py-0.5 font-label-caps text-label-caps text-on-surface-variant"
              title="Read-only: editing a truncated or non-UTF-8 file could corrupt it"
            >
              READ-ONLY
            </span>
          )}
          {dirty && (
            <span
              className="rounded border border-amber-500/40 bg-amber-500/15 px-2 py-0.5 font-label-caps text-label-caps text-amber-300"
              title="Unsaved edits"
            >
              UNSAVED CHANGES
            </span>
          )}
          <button
            type="button"
            onClick={() => setShowMetadata((v) => !v)}
            title={
              showMetadata
                ? "Hide CL index metadata"
                : "Edit CL index metadata (description + tags)"
            }
            className="inline-flex items-center gap-1.5 rounded border border-outline-variant bg-transparent px-3 py-1.5 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:bg-surface-container-high hover:text-on-surface"
          >
            {showMetadata ? "Hide metadata" : "Edit metadata"}
            {!showMetadata && metadataDirty && (
              <span
                className="h-1.5 w-1.5 rounded-full bg-amber-400"
                aria-label="unsaved metadata edits"
              />
            )}
          </button>
          <button
            type="button"
            disabled={!dirty || saving}
            onClick={handleSave}
            title={
              readOnly
                ? "Read-only file — cannot save"
                : dirty
                  ? "Save changes to disk"
                  : "No unsaved changes"
            }
            className="inline-flex items-center gap-1.5 rounded border border-primary bg-primary px-3 py-1.5 font-code-sm text-code-sm text-on-primary transition-colors hover:bg-primary-fixed disabled:cursor-not-allowed disabled:opacity-40"
          >
            <SaveIcon />
            {saving ? "Saving…" : "Save Changes"}
          </button>
        </div>
      </header>

      {saveError && (
        <p className="flex-shrink-0 border-b border-outline-variant px-4 py-1.5 font-code-sm text-code-sm text-error">
          Save failed: {saveError}{" "}
          <button
            onClick={() => setSaveError(null)}
            className="underline hover:text-on-surface"
          >
            dismiss
          </button>
        </p>
      )}

      <div className="min-h-0 flex-1 overflow-hidden bg-surface-container-low">
        {isFetching && !fileContent ? (
          <p className="px-4 py-3 font-code-sm text-code-sm text-on-surface-variant">
            Loading…
          </p>
        ) : fileError ? (
          <p className="px-4 py-3 font-code-sm text-code-sm text-error">
            Failed to read: {String(fileError.message ?? fileError)}
          </p>
        ) : fileContent && draft != null ? (
          <CodeView value={draft} onChange={setDraft} readOnly={readOnly} />
        ) : null}
      </div>

      {/* Kept mounted but visually hidden when collapsed, so an in-progress
          metadata edit isn't lost when the user toggles it shut. */}
      <div className={cn(!showMetadata && "hidden")}>
        <DescriptionEditor
          project={tab.project}
          filePath={tab.filePath}
          initial={entry?.description ?? ""}
          tags={entry?.tags ?? null}
          onSaved={onRefetchIndex}
          onDirtyChange={setMetadataDirty}
        />
      </div>
    </div>
  );
}

// ============================================================================
// CodeView — content + line-number gutter (no syntax highlighting in v1)
// ============================================================================

function CodeView({
  value,
  onChange,
  readOnly,
}: {
  value: string;
  onChange: (v: string) => void;
  readOnly: boolean;
}) {
  const lineCount = useMemo(
    () => Math.max(1, value.split("\n").length),
    [value],
  );
  const gutterWidthCh = String(lineCount).length + 1; // +1 for breathing room
  // Keep the gutter scroll-locked to the textarea so line numbers stay aligned.
  const gutterRef = useRef<HTMLDivElement>(null);

  return (
    <div className="flex h-full font-code-sm text-code-sm">
      <div
        ref={gutterRef}
        className="select-none overflow-hidden border-r border-outline-variant/30 px-3 py-3 text-right text-on-surface-variant/60"
        style={{ minWidth: `${gutterWidthCh}ch` }}
        aria-hidden
      >
        {Array.from({ length: lineCount }, (_, i) => (
          <div key={i} className="leading-relaxed">
            {i + 1}
          </div>
        ))}
      </div>
      <textarea
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onScroll={(e) => {
          if (gutterRef.current) {
            gutterRef.current.scrollTop = e.currentTarget.scrollTop;
          }
        }}
        readOnly={readOnly}
        wrap="off"
        spellCheck={false}
        aria-label="File content editor"
        className="h-full flex-1 resize-none overflow-auto whitespace-pre border-0 bg-transparent px-4 py-3 leading-relaxed text-on-surface caret-primary outline-none focus:ring-0"
      />
    </div>
  );
}

// ============================================================================
// EmptyEditor — shown when no tabs are open
// ============================================================================

function EmptyEditor() {
  return (
    <div className="flex min-h-0 flex-1 items-center justify-center bg-background">
      <div className="text-center">
        <p className="font-headline-md text-headline-md text-on-surface-variant">
          No file open
        </p>
        <p className="mt-1 font-code-sm text-code-sm text-on-surface-variant/60">
          Pick a file from the sidebar to open it in a tab.
        </p>
      </div>
    </div>
  );
}

// ============================================================================
// DescriptionEditor — wraps the `cl_set_description` Tauri command
// (snake_case backend; tauri-specta camelCases the IPC keys). Saves are
// idempotent — backend treats an unknown (project, file_path) as upsert.
// ============================================================================

function DescriptionEditor({
  project,
  filePath,
  initial,
  tags,
  onSaved,
  onDirtyChange,
}: {
  project: string;
  filePath: string;
  initial: string;
  tags: string | null;
  onSaved: () => void;
  onDirtyChange?: (dirty: boolean) => void;
}) {
  const seedKey = `${project}/${filePath}`;
  const [seed, setSeed] = useState(seedKey);
  const [desc, setDesc] = useState(initial);
  const [tagsStr, setTagsStr] = useState(tags ?? "");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  if (seed !== seedKey) {
    setSeed(seedKey);
    setDesc(initial);
    setTagsStr(tags ?? "");
    setError(null);
  }

  const initialTagsStr = tags ?? "";
  const dirty = desc !== initial || tagsStr !== initialTagsStr;
  useEffect(() => {
    onDirtyChange?.(dirty);
  }, [dirty, onDirtyChange]);

  const handleSave = async () => {
    if (!dirty || saving) return;
    setSaving(true);
    setError(null);
    try {
      await invoke("cl_set_description", {
        project,
        filePath,
        description: desc,
        tags: tagsStr.trim() ? tagsStr.trim() : null,
      });
      onSaved();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="flex-shrink-0 border-t border-outline-variant bg-surface-container px-4 py-3">
      <p className="mb-2 font-label-caps text-label-caps text-on-surface-variant/70">
        Metadata · CL index entry
      </p>
      <label className="block">
        <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
          Description
        </span>
        <textarea
          rows={2}
          value={desc}
          onChange={(e) => setDesc(e.target.value)}
          placeholder="One-line description shown in the CL index."
          className="w-full resize-y rounded border border-outline-variant bg-surface-container-lowest px-2 py-1 font-code-sm text-code-sm text-on-surface placeholder:text-on-surface-variant focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
        />
      </label>
      <label className="mt-3 block">
        <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
          Tags
        </span>
        <input
          type="text"
          value={tagsStr}
          onChange={(e) => setTagsStr(e.target.value)}
          placeholder="(optional, comma-separated)"
          className={terminalInputClass}
        />
      </label>
      {error && (
        <p className="mt-2 font-code-sm text-code-sm text-error">
          Save failed: {error}{" "}
          <button
            onClick={() => setError(null)}
            className="underline hover:text-on-error-container"
          >
            dismiss
          </button>
        </p>
      )}
      <div className="mt-3 flex items-center justify-end gap-2">
        <button
          type="button"
          disabled={!dirty || saving}
          onClick={() => {
            setDesc(initial);
            setTagsStr(initialTagsStr);
            setError(null);
          }}
          className="rounded border border-outline-variant bg-transparent px-3 py-1 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:bg-surface-container-high hover:text-on-surface disabled:opacity-50"
        >
          Reset
        </button>
        <button
          type="button"
          disabled={!dirty || saving}
          onClick={handleSave}
          className="rounded border border-outline-variant bg-surface-container-high px-3 py-1 font-code-sm text-code-sm text-on-surface transition-colors hover:bg-surface-container-highest disabled:opacity-50"
        >
          {saving ? "Saving…" : "Save metadata"}
        </button>
      </div>
    </div>
  );
}
