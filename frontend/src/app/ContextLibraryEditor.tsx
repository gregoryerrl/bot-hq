import { memo, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTauriQuery, useTauriMutation, errorMessage } from "../hooks/useInvoke";
import { useServerDraft } from "../hooks/useServerDraft";
import { cn } from "../lib/cn";
import type {
  ClFileContentView,
  ClFolderView,
  ClIndexEntryView,
  Policy,
  ProjectView,
  RetrievalStatsView,
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
import { SegToggle } from "../components/ui/SegToggle";

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
  onProjectGone: (name: string, replacement?: string) => void;
}

function EditorAreaImpl({
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
  onProjectGone,
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
      ) : activeTab.kind === "proposals" ? (
        <ProposalQueue
          key={tabKey(activeTab)}
          project={activeTab.project}
          onProjectChanged={onProjectChanged}
        />
      ) : activeTab.kind === "measurement" ? (
        <MeasurementView
          key={tabKey(activeTab)}
          project={activeTab.project}
        />
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
          onProjectGone={onProjectGone}
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

// Memoized (O6): EditorArea is unrelated to CL search typing / sidebar drag, so
// it skips re-render while those churn the parent — its callbacks are stabilized
// (useCallback) in ContextLibrary and its data props are referentially stable
// between index refetches.
export const EditorArea = memo(EditorAreaImpl);

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
        <SegToggle<"form" | "raw">
          value={view}
          onChange={setView}
          className="shrink-0"
          options={[
            { value: "form", label: "Form", tone: "warn" },
            { value: "raw", label: "Raw YAML", tone: "warn" },
          ]}
        />
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

  const { draft, setDraft, dirty } = useServerDraft<Policy>(server ?? {});

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
        const path =
          t.kind === "file"
            ? t.filePath
            : t.kind === "folder"
              ? t.folderPath
              : "proposals";
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
  // remounts per file; a one-shot seed handles the first load.
  const [draft, setDraft] = useState<string | null>(null);
  if (draft === null && fileContent) setDraft(fileContent.content);

  // Live-refresh the open file when it changes on disk (external edit, agent
  // write, cloud sync) — the `cl:changed` watcher event invalidates this query,
  // and we adopt the new server content ONLY when the editor is clean. A dirty
  // editor keeps the user's unsaved text (last-write-wins on save).
  // `syncedContent` is the server content the draft was last synced to.
  const syncedContent = useRef<string | null>(null);
  useEffect(() => {
    if (!fileContent) return;
    const old = syncedContent.current;
    const incoming = fileContent.content;
    if (old === null) {
      syncedContent.current = incoming; // record the baseline after the first load
      return;
    }
    if (incoming === old) return; // no external change
    syncedContent.current = incoming;
    // Clean (draft still equals the last-synced content) → adopt; dirty → keep.
    setDraft((prev) => (prev === old ? incoming : prev));
  }, [fileContent]);

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
                    <span className="text-warning">
                      truncated to 1 MB
                    </span>
                  </>
                )}
                {fileContent.binary && (
                  <>
                    <span className="mx-2 text-on-surface-variant/60">·</span>
                    <span className="text-warning">binary / non-UTF-8</span>
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
              className="rounded border border-warning/40 bg-warning/15 px-2 py-0.5 font-label-caps text-label-caps text-warning"
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
                className="h-1.5 w-1.5 rounded-full bg-warning"
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
// MeasurementView — Stage-4b CL retrieval telemetry (tokens-per-task et al.)
// ============================================================================

function StatTile({
  label,
  value,
  hint,
  accent,
  warn,
}: {
  label: string;
  value: string;
  hint?: string;
  accent?: boolean;
  warn?: boolean;
}) {
  return (
    <div
      className={cn(
        "rounded border border-outline-variant bg-surface-container-low px-3 py-2.5",
        accent && "border-primary/40 bg-primary/5",
      )}
    >
      <p className="font-label-caps text-label-caps text-on-surface-variant">
        {label}
      </p>
      <p
        className={cn(
          "mt-1 font-headline-md text-headline-md tabular-nums",
          warn ? "text-error" : accent ? "text-primary" : "text-on-surface",
        )}
      >
        {value}
      </p>
      {hint && (
        <p className="mt-0.5 font-code-sm text-code-sm text-on-surface-variant/60">
          {hint}
        </p>
      )}
    </div>
  );
}

function MeasurementView({ project }: { project: string }) {
  const {
    data: stats,
    isFetching,
    error,
    refetch,
  } = useTauriQuery<RetrievalStatsView>("cl_retrieval_stats", {
    project,
    since: null,
  });

  const pct = (r: number) => `${(r * 100).toFixed(1)}%`;
  const num = (n: number) => Math.round(n).toLocaleString();

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-background">
      <header className="flex flex-shrink-0 items-center justify-between gap-3 border-b border-outline-variant bg-surface-container-low px-4 py-2">
        <div className="min-w-0">
          <p className="font-headline-md text-headline-md text-on-surface">
            Retrieval measurement
          </p>
          <p className="font-code-sm text-code-sm text-on-surface-variant">
            {project} · cl_retrieve telemetry (all time)
          </p>
        </div>
        <button
          type="button"
          onClick={() => refetch()}
          className="rounded border border-outline-variant px-2 py-0.5 font-label-caps text-label-caps text-on-surface-variant hover:text-on-surface"
        >
          Refresh
        </button>
      </header>

      <div className="min-h-0 flex-1 overflow-auto px-4 py-4">
        {isFetching && !stats ? (
          <p className="font-code-sm text-code-sm text-on-surface-variant">
            Loading measurement…
          </p>
        ) : error ? (
          <p className="font-code-sm text-code-sm text-error">
            Failed to load measurement: {error.message}
          </p>
        ) : !stats || stats.event_count === 0 ? (
          <div className="flex h-full items-center justify-center text-center">
            <div>
              <p className="font-headline-md text-headline-md text-on-surface-variant">
                No retrievals logged yet
              </p>
              <p className="mt-1 font-code-sm text-code-sm text-on-surface-variant/60">
                Every cl_retrieve call an agent makes for {project} is logged
                here; the numbers appear once retrievals happen.
              </p>
            </div>
          </div>
        ) : (
          <div className="space-y-4">
            <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
              <StatTile
                label="Tokens / session"
                value={num(stats.avg_tokens_per_session)}
                hint="tokens-per-task proxy"
                accent
              />
              <StatTile
                label="Tokens / retrieval"
                value={num(stats.avg_tokens_per_event)}
              />
              <StatTile label="Retrievals" value={num(stats.event_count)} />
              <StatTile label="Sessions" value={num(stats.distinct_sessions)} />
              <StatTile
                label="Atoms returned"
                value={num(stats.total_atoms)}
              />
              <StatTile label="Total tokens" value={num(stats.total_tokens)} />
              <StatTile
                label="Stale-hit rate"
                value={pct(stats.stale_hit_rate)}
                hint={`${num(stats.stale_hits)} ⚠ atoms`}
                warn={stats.stale_hit_rate > 0}
              />
              <StatTile
                label="Retrieval-miss rate"
                value={pct(stats.empty_return_rate)}
                hint={`${num(stats.empty_returns)} empty`}
                warn={stats.empty_return_rate > 0}
              />
            </div>
            <p className="font-code-sm text-code-sm text-on-surface-variant/70">
              Low tokens/session with a low miss rate means retrieval is earning
              its keep. A rising stale-hit rate means atoms cite drifted code —
              time to re-index or prune.
            </p>
          </div>
        )}
      </div>
    </div>
  );
}

// ============================================================================
// ProposalQueue — project-scoped CL proposal review docket
// ============================================================================

type ClProposalView = {
  id: number;
  proposal_uid: string;
  project: string;
  file_path: string;
  kind: "add" | "correct" | "delete" | string;
  target_excerpt: string | null;
  proposed_body: string;
  evidence: string;
  status: string;
  proposed_by: string;
  session_id: string | null;
  created_at: string;
  updated_at: string;
};

function ProposalQueue({
  project,
  onProjectChanged,
}: {
  project: string;
  onProjectChanged: () => void;
}) {
  const {
    data: proposals = [],
    isFetching,
    error,
    refetch,
  } = useTauriQuery<ClProposalView[]>("cl_list_proposals", {
    project,
    status: "open",
  });
  const [busyUids, setBusyUids] = useState<Set<string>>(() => new Set());
  const [actionError, setActionError] = useState<string | null>(null);

  const runProposalAction = async (
    proposal: ClProposalView,
    action: "approve" | "reject",
  ) => {
    setBusyUids((prev) => new Set(prev).add(proposal.proposal_uid));
    setActionError(null);
    try {
      await invoke(
        action === "approve" ? "cl_approve_proposal" : "cl_reject_proposal",
        { proposalUid: proposal.proposal_uid },
      );
      await refetch();
      onProjectChanged();
    } catch (e) {
      setActionError(errorMessage(e));
    } finally {
      setBusyUids((prev) => {
        const next = new Set(prev);
        next.delete(proposal.proposal_uid);
        return next;
      });
    }
  };

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-background">
      <header className="flex flex-shrink-0 items-center justify-between gap-3 border-b border-outline-variant bg-surface-container-low px-4 py-2">
        <div className="min-w-0">
          <p className="font-headline-md text-headline-md text-on-surface">
            Proposal docket
          </p>
          <p className="font-code-sm text-code-sm text-on-surface-variant">
            {project} · open CL proposals
          </p>
        </div>
        <span className="rounded border border-secondary/40 bg-secondary/10 px-2 py-0.5 font-label-caps text-label-caps text-secondary">
          {proposals.length} OPEN
        </span>
      </header>

      {actionError && (
        <p className="flex-shrink-0 border-b border-outline-variant px-4 py-1.5 font-code-sm text-code-sm text-error">
          Proposal action failed: {actionError}{" "}
          <button
            onClick={() => setActionError(null)}
            className="underline hover:text-on-surface"
          >
            dismiss
          </button>
        </p>
      )}

      <div className="min-h-0 flex-1 overflow-auto px-4 py-4">
        {isFetching && proposals.length === 0 ? (
          <p className="font-code-sm text-code-sm text-on-surface-variant">
            Loading proposals…
          </p>
        ) : error ? (
          <p className="font-code-sm text-code-sm text-error">
            Failed to load proposals: {error.message}
          </p>
        ) : proposals.length === 0 ? (
          <div className="flex h-full items-center justify-center text-center">
            <div>
              <p className="font-headline-md text-headline-md text-on-surface-variant">
                No open proposals
              </p>
              <p className="mt-1 font-code-sm text-code-sm text-on-surface-variant/60">
                Agents can file CL edits with cl_propose; they will appear here.
              </p>
            </div>
          </div>
        ) : (
          <div className="space-y-3">
            {proposals.map((proposal) => {
              const approveUnsupported = proposal.kind === "delete";
              const busy = busyUids.has(proposal.proposal_uid);
              return (
                <article
                  key={proposal.proposal_uid}
                  className="grid grid-cols-[4px_minmax(0,1fr)] overflow-hidden rounded border border-outline-variant bg-surface-container-low"
                >
                  <div className="bg-secondary" aria-hidden />
                  <div className="min-w-0 p-4">
                    <div className="flex flex-wrap items-start justify-between gap-3">
                      <div className="min-w-0">
                        <p className="truncate font-code-sm text-code-sm text-on-surface">
                          {proposal.file_path}
                        </p>
                        <p className="mt-0.5 font-code-sm text-code-sm text-on-surface-variant">
                          proposed by {proposal.proposed_by}
                        </p>
                      </div>
                      <div className="flex shrink-0 items-center gap-2">
                        <span className="rounded border border-secondary/40 bg-secondary/10 px-2 py-0.5 font-label-caps text-label-caps text-secondary">
                          {proposal.kind.toUpperCase()}
                        </span>
                        <span className="rounded border border-outline-variant bg-surface-container-high px-2 py-0.5 font-label-caps text-label-caps text-on-surface-variant">
                          {proposal.status.toUpperCase()}
                        </span>
                      </div>
                    </div>

                    <div className="mt-3 grid gap-3 lg:grid-cols-2">
                      <section>
                        <p className="mb-1 font-label-caps text-label-caps text-on-surface-variant">
                          Evidence
                        </p>
                        <p className="rounded border border-outline-variant/60 bg-surface-container-lowest px-3 py-2 font-body-md text-body-md text-on-surface">
                          {proposal.evidence}
                        </p>
                      </section>
                      <section>
                        <p className="mb-1 font-label-caps text-label-caps text-on-surface-variant">
                          Proposed body
                        </p>
                        <pre className="max-h-36 overflow-auto rounded border border-outline-variant/60 bg-surface-container-lowest px-3 py-2 font-code-sm text-code-sm text-on-surface whitespace-pre-wrap">
                          {proposal.proposed_body || "(no body — delete proposal)"}
                        </pre>
                      </section>
                    </div>

                    {proposal.target_excerpt && (
                      <section className="mt-3">
                        <p className="mb-1 font-label-caps text-label-caps text-on-surface-variant">
                          Target excerpt
                        </p>
                        <pre className="max-h-28 overflow-auto rounded border border-outline-variant/60 bg-surface-container-lowest px-3 py-2 font-code-sm text-code-sm text-on-surface-variant whitespace-pre-wrap">
                          {proposal.target_excerpt}
                        </pre>
                      </section>
                    )}

                    {approveUnsupported ? (
                      <p className="mt-3 rounded border border-warning/40 bg-warning/10 px-3 py-2 font-code-sm text-code-sm text-warning">
                        Delete approval is deferred for this MVP. Reject this proposal or leave it open.
                      </p>
                    ) : proposal.kind === "correct" ? (
                      <p className="mt-3 rounded border border-primary/30 bg-primary/10 px-3 py-2 font-code-sm text-code-sm text-primary">
                        Approving replaces the entire file with the proposed body.
                      </p>
                    ) : null}

                    <div className="mt-4 flex justify-end gap-2">
                      <button
                        type="button"
                        onClick={() => runProposalAction(proposal, "reject")}
                        disabled={busy}
                        aria-label="Reject proposal"
                        className="rounded border border-outline-variant bg-transparent px-3 py-1.5 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:bg-surface-container-high hover:text-on-surface disabled:opacity-50"
                      >
                        {busy ? "Working…" : "Reject"}
                      </button>
                      <button
                        type="button"
                        onClick={() => runProposalAction(proposal, "approve")}
                        disabled={busy || approveUnsupported}
                        aria-label={approveUnsupported ? "Approve unsupported" : "Approve proposal"}
                        className="rounded border border-primary bg-primary px-3 py-1.5 font-code-sm text-code-sm text-on-primary transition-colors hover:bg-primary-fixed disabled:cursor-not-allowed disabled:opacity-40"
                      >
                        {approveUnsupported ? "Unsupported" : busy ? "Working…" : "Approve"}
                      </button>
                    </div>
                  </div>
                </article>
              );
            })}
          </div>
        )}
      </div>
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
