import { useCallback, useEffect, useRef, useState, type ReactElement } from "react";
import { Link, useBlocker } from "react-router-dom";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useTauriQuery, useTauriMutation, errorMessage } from "../hooks/useInvoke";
import { useServerDraft } from "../hooks/useServerDraft";
import { Button } from "../components/ui/Button";
import { SubTabButton } from "../components/SubTabButton";
import { cn } from "../lib/cn";
import { formatTimestamp } from "../lib/time";
import { terminalInputClass, FieldLabel } from "./contextLibraryShared";
import { SaveIcon } from "./contextLibraryShared";
import { WrenchIcon, EyeIcon, GearIcon } from "../components/icons";
import { ClaudeConfigPanel } from "./ClaudeConfig";
import { ModelsPanel } from "./ModelsPanel";
import { ViolationsPanel } from "./ViolationsPanel";
import { PolicyForm } from "../components/PolicyForm";
import { GatedKeywordList } from "../components/GatedKeywordList";
import type {
  AgentConfigView,
  GatedKeyword,
  ModelView,
  Policy,
  SessionInfo,
  UpdateInfo,
} from "../lib/bindings";

type SettingsSubTab =
  | "agents"
  | "models"
  | "claude"
  | "toolgate"
  | "policy"
  | "violations"
  | "archive"
  | "updates";

/**
 * Settings is a tabbed container. The existing per-agent model/auth cards
 * ("Agents") and the global Tool-Gate list ("Tool Gate") sit alongside the new
 * "Claude Config" subtab, which surfaces the Claude Code config the agents
 * inherit + the per-agent override layer. All three stay mounted (toggled with
 * `hidden`) so in-progress edits — and the Agents route-blocker — survive a
 * subtab switch.
 */
export function Settings() {
  const [tab, setTab] = useState<SettingsSubTab>("agents");
  // Lazy-mount-then-keep: a panel mounts only once its tab has been visited, then
  // STAYS mounted (CSS `hidden` when inactive) so in-progress edits survive a
  // subtab switch. This skips firing the queries of tabs the user never opens —
  // the old code mounted all 6 panels (and all their queries) on first visit.
  const [visited, setVisited] = useState<Set<SettingsSubTab>>(
    () => new Set<SettingsSubTab>(["agents"]),
  );
  const select = (t: SettingsSubTab) => {
    setTab(t);
    setVisited((v) => (v.has(t) ? v : new Set(v).add(t)));
  };
  return (
    <div className="flex h-full flex-col bg-background">
      <div className="flex shrink-0 items-center gap-1 border-b border-outline-variant px-4">
        <SubTabButton active={tab === "agents"} onClick={() => select("agents")}>
          Agents
        </SubTabButton>
        <SubTabButton active={tab === "models"} onClick={() => select("models")}>
          Models
        </SubTabButton>
        <SubTabButton active={tab === "claude"} onClick={() => select("claude")}>
          Claude Config
        </SubTabButton>
        <SubTabButton
          active={tab === "toolgate"}
          onClick={() => select("toolgate")}
        >
          Tool Gate
        </SubTabButton>
        <SubTabButton active={tab === "policy"} onClick={() => select("policy")}>
          Policy
        </SubTabButton>
        <SubTabButton
          active={tab === "violations"}
          onClick={() => select("violations")}
        >
          Violations
        </SubTabButton>
        <SubTabButton
          active={tab === "archive"}
          onClick={() => select("archive")}
        >
          Archive
        </SubTabButton>
        <SubTabButton
          active={tab === "updates"}
          onClick={() => select("updates")}
        >
          Updates
        </SubTabButton>
      </div>
      <div className="min-h-0 flex-1">
        <div className={cn("h-full", tab !== "agents" && "hidden")}>
          {visited.has("agents") && <AgentsPanel />}
        </div>
        <div className={cn("h-full", tab !== "models" && "hidden")}>
          {visited.has("models") && <ModelsPanel />}
        </div>
        <div className={cn("h-full", tab !== "claude" && "hidden")}>
          {visited.has("claude") && <ClaudeConfigPanel />}
        </div>
        <div className={cn("h-full", tab !== "toolgate" && "hidden")}>
          {visited.has("toolgate") && <ToolGatePanel />}
        </div>
        <div className={cn("h-full", tab !== "policy" && "hidden")}>
          {visited.has("policy") && <GlobalPolicyPanel />}
        </div>
        <div className={cn("h-full", tab !== "violations" && "hidden")}>
          {visited.has("violations") && <ViolationsPanel />}
        </div>
        <div className={cn("h-full", tab !== "archive" && "hidden")}>
          {visited.has("archive") && <ArchivePanel />}
        </div>
        <div className={cn("h-full", tab !== "updates" && "hidden")}>
          {visited.has("updates") && <UpdatesPanel />}
        </div>
      </div>
    </div>
  );
}

function ToolGatePanel() {
  return (
    <div className="mx-auto h-full max-w-7xl overflow-y-auto overflow-x-hidden px-6 py-6">
      <ToolGateSection />
    </div>
  );
}

// ============================================================================
// Policy — global tier (general-policy.yaml), the base every project + session
// inherits from at spawn. Project overrides live in the Context Library; the
// per-session snapshot lives in the session gear panel.
// ============================================================================

function GlobalPolicyPanel() {
  const { data: server, refetch, isLoading } = useTauriQuery<Policy>(
    "get_general_policy",
  );
  const save = useTauriMutation<void, { policy: Policy }>("set_general_policy");

  const { draft, setDraft, dirty } = useServerDraft<Policy>(server ?? {});

  const onSave = async () => {
    await save.mutateAsync({ policy: draft });
    refetch();
  };

  return (
    <div className="mx-auto h-full max-w-4xl overflow-y-auto overflow-x-hidden px-6 py-6">
      <div className="mb-6 flex items-start justify-between gap-4">
        <div>
          <h1 className="font-headline-lg text-headline-lg text-on-surface">
            Global Policy
          </h1>
          <p className="mt-1 max-w-prose font-body-md text-body-md text-on-surface-variant">
            The base policy every project and session inherits at spawn
            (<code>general-policy.yaml</code>). Projects can tighten it in the
            Context Library; a live session can override it in the gear panel.
          </p>
        </div>
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
        <div className="rounded-lg border border-outline-variant bg-surface-container p-5">
          <PolicyForm value={draft} onChange={setDraft} disabled={save.isPending} />
        </div>
      )}
      {save.error && (
        <p className="mt-4 rounded border border-error/40 bg-error-container/20 px-3 py-2 font-code-sm text-code-sm text-on-error-container">
          Save failed: {save.error.message}
        </p>
      )}
    </div>
  );
}

function AgentsPanel() {
  const { data: configs = [], refetch, isLoading } = useTauriQuery<
    AgentConfigView[]
  >("list_agent_configs");
  const { data: models = [] } = useTauriQuery<ModelView[]>("list_models");
  const upsert = useTauriMutation<void, { cfg: AgentConfigView }>(
    "upsert_agent_config",
  );

  // Per-agent dirty tracking. `dirtyRef` is the source of truth (avoids
  // re-renders on every keystroke); `dirtyCount` mirrors size so the
  // blocker's gate-fn closure stays current.
  const dirtyRef = useRef<Set<string>>(new Set());
  const [dirtyCount, setDirtyCount] = useState(0);

  const setDirty = useCallback((agentName: string, dirty: boolean) => {
    const prev = dirtyRef.current.size;
    if (dirty) {
      dirtyRef.current.add(agentName);
    } else {
      dirtyRef.current.delete(agentName);
    }
    const next = dirtyRef.current.size;
    if ((prev === 0) !== (next === 0)) setDirtyCount(next);
  }, []);

  const blocker = useBlocker(
    // eslint-disable-next-line react-hooks/exhaustive-deps
    useCallback(() => dirtyRef.current.size > 0, [dirtyCount]),
  );

  // Save-all uses a counter as a fan-out signal: incrementing it triggers
  // every AgentRow's effect, which checks its own dirty state and saves
  // if needed. Avoids lifting draft state out of AgentRow.
  const [saveAllSignal, setSaveAllSignal] = useState(0);

  return (
    <div className="mx-auto h-full max-w-7xl overflow-y-auto overflow-x-hidden px-6 py-6">
      {blocker.state === "blocked" && (
        <div className="mb-4 flex items-center gap-3 rounded-lg border border-error/40 bg-error-container/20 px-4 py-3">
          <p className="flex-1 font-code-sm text-code-sm text-on-error-container">
            You have unsaved changes. Leave without saving?
          </p>
          <Button variant="ghost" size="sm" onClick={() => blocker.reset()}>
            Stay
          </Button>
          <Button variant="danger" size="sm" onClick={() => blocker.proceed()}>
            Leave
          </Button>
        </div>
      )}
      <div className="mb-6 flex items-start justify-between gap-4">
        <div>
          <h1 className="font-headline-lg text-headline-lg text-on-surface">
            Agent Configuration
          </h1>
          <p className="mt-1 max-w-prose font-body-md text-body-md text-on-surface-variant">
            Manage connection parameters for individual orchestration agents.
          </p>
        </div>
        {dirtyCount > 0 && (
          <button
            type="button"
            onClick={() => setSaveAllSignal((n) => n + 1)}
            disabled={upsert.isPending}
            className="inline-flex items-center gap-2 rounded border border-primary bg-primary px-3 py-1.5 font-code-sm text-code-sm text-on-primary transition-colors hover:bg-primary-fixed disabled:opacity-50"
          >
            <SaveIcon />
            Save all ({dirtyCount})
          </button>
        )}
      </div>
      {isLoading ? (
        <div className="grid grid-cols-1 gap-gutter xl:grid-cols-2">
          {[0, 1].map((i) => (
            <div
              key={i}
              className="h-64 animate-pulse rounded-lg border border-outline-variant bg-surface-container"
            />
          ))}
        </div>
      ) : (
        <div className="grid grid-cols-1 gap-gutter xl:grid-cols-2">
          {configs.map((c) => (
            <AgentCard
              key={c.agent_name}
              cfg={c}
              models={models}
              onSave={async (next) => {
                await upsert.mutateAsync({ cfg: next });
                setDirty(c.agent_name, false);
                refetch();
              }}
              onDirtyChange={(dirty) => setDirty(c.agent_name, dirty)}
              isSaving={upsert.isPending}
              saveAllSignal={saveAllSignal}
            />
          ))}
        </div>
      )}
      <SessionDefaults />
    </div>
  );
}

/** App-wide defaults applied at session create (not per-agent config). */
function SessionDefaults() {
  const { data: worktreeDefault, refetch } = useTauriQuery<string | null>(
    "get_app_setting",
    { key: "worktree_default" },
  );
  const setAppSetting = useTauriMutation<void, { key: string; value: string }>(
    "set_app_setting",
  );
  return (
    <section className="mt-gutter rounded-lg border border-outline-variant bg-surface-container p-4">
      <h2 className="font-headline-md text-headline-md text-on-surface">
        Session defaults
      </h2>
      <label className="mt-3 flex items-center gap-2">
        <input
          type="checkbox"
          checked={worktreeDefault !== "0"}
          onChange={async (e) => {
            await setAppSetting.mutateAsync({
              key: "worktree_default",
              value: e.target.checked ? "1" : "0",
            });
            refetch();
          }}
          className="size-4 accent-primary"
        />
        <span className="font-body-md text-body-md text-on-surface">
          Run repo-backed sessions in isolated git worktrees
        </span>
      </label>
      {setAppSetting.error && (
        <p className="mt-2 inline-block rounded border border-error/40 bg-error-container/20 px-2 py-1 font-code-sm text-code-sm text-on-error-container">
          Couldn’t save: {setAppSetting.error.message}
        </p>
      )}
      <p className="mt-1 font-code-sm text-code-sm text-on-surface-variant">
        Each session gets its own checkout on branch{" "}
        <code className="text-on-surface">bothq/&lt;session-id&gt;</code>, so
        several sessions can work the same project in parallel. Clean worktrees
        are removed at close; anything uncommitted is kept. The New-session
        dialog can override this per session.
      </p>
    </section>
  );
}

function AgentCard({
  cfg,
  models,
  onSave,
  onDirtyChange,
  isSaving,
  saveAllSignal,
}: {
  cfg: AgentConfigView;
  models: ModelView[];
  onSave: (next: AgentConfigView) => Promise<void>;
  onDirtyChange: (dirty: boolean) => void;
  isSaving?: boolean;
  saveAllSignal: number;
}) {
  const [draft, setDraft] = useState(cfg);
  const [saved, setSaved] = useState(false);
  // Inline per-card save error so a rejected upsert (button or Save-all) gives
  // feedback instead of an unhandled rejection. Scoped to the card that failed.
  const [saveError, setSaveError] = useState<string | null>(null);
  const dirty = JSON.stringify(draft) !== JSON.stringify(cfg);

  // Rain-only "disable by default" preference (app_settings). The hooks run for
  // every card (React Query dedupes) but only Rain renders the checkbox.
  const { data: rainDisabledDefault, refetch: refetchRainDefault } =
    useTauriQuery<string | null>("get_app_setting", {
      key: "rain_disabled_default",
    });
  const setAppSetting = useTauriMutation<void, { key: string; value: string }>(
    "set_app_setting",
  );

  // Which saved model (if any) the current config corresponds to. Exact match
  // on the spawn-relevant fields so the dropdown reflects the agent's model.
  const selectedModelId =
    models.find(
      (m) =>
        m.provider === draft.provider &&
        m.model_name === draft.model_name &&
        (m.base_url ?? "") === (draft.base_url ?? ""),
    )?.id ?? "";

  // Save-all fan-out: parent increments saveAllSignal; each dirty row
  // triggers its own save. Skipping initial mount via a ref guards against
  // saving on first render when saveAllSignal=0.
  const lastSeenSignal = useRef(saveAllSignal);
  useEffect(() => {
    if (saveAllSignal === lastSeenSignal.current) return;
    lastSeenSignal.current = saveAllSignal;
    if (!dirty) return;
    setSaveError(null);
    onSave(draft)
      .then(() => setSaved(true))
      .catch((e) => setSaveError(errorMessage(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [saveAllSignal]);

  // Auto-clear the "Saved ✓" badge after 2s so it doesn't linger forever.
  useEffect(() => {
    if (!saved) return;
    const id = setTimeout(() => setSaved(false), 2000);
    return () => clearTimeout(id);
  }, [saved]);

  // Push dirty state up to Settings so the route-blocker knows. When the
  // user resumes editing post-save, clear the green badge so the staleness
  // is unambiguous.
  const prevDirty = useRef(dirty);
  if (prevDirty.current !== dirty) {
    prevDirty.current = dirty;
    onDirtyChange(dirty);
    if (dirty) {
      setSaved(false);
      setSaveError(null);
    }
  }

  return (
    <section
      className={cn(
        "flex flex-col rounded-lg border bg-surface-container p-4",
        roleBorder(cfg.agent_name),
      )}
    >
      {/* Header: icon + name + status badges + role chip */}
      <div className="mb-4 flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-2">
          <span className="flex items-center leading-none" aria-hidden>
            {agentIcon(cfg.agent_name)}
          </span>
          <h2 className="truncate font-headline-md text-headline-md capitalize text-on-surface">
            {cfg.agent_name}
          </h2>
          {dirty && (
            <span className="shrink-0 rounded border border-warning/40 bg-warning/15 px-1.5 py-0.5 font-label-caps text-label-caps text-warning">
              Unsaved
            </span>
          )}
          {saved && !dirty && (
            <span className="shrink-0 rounded border border-success/40 bg-success/15 px-1.5 py-0.5 font-label-caps text-label-caps text-success">
              Saved ✓
            </span>
          )}
        </div>
      </div>

      {/* Form fields */}
      <div className="flex flex-1 flex-col gap-4">
        {/* Model is chosen from the saved-model registry (Models tab). The
            provider / endpoint / credential all live on the model — there are
            no per-agent free-text fields. */}
        <label className="block">
          <FieldLabel>Model</FieldLabel>
          <select
            value={selectedModelId}
            onChange={(e) => {
              const m = models.find((x) => x.id === e.target.value);
              if (m) {
                setDraft({
                  ...draft,
                  provider: m.provider,
                  model_name: m.model_name,
                  base_url: m.base_url,
                  auth_token: m.auth_token,
                });
              }
            }}
            className="w-full rounded border border-outline-variant bg-surface-container-lowest px-2 py-1.5 font-code-sm text-code-sm text-on-surface focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
          >
            {selectedModelId === "" && (
              <option value="" disabled>
                {models.length === 0 ? "(no saved models)" : "(select a model)"}
              </option>
            )}
            {models.map((m) => (
              <option key={m.id} value={m.id}>
                {m.display_name}
                {m.model_name ? ` — ${m.model_name}` : ""}
              </option>
            ))}
          </select>
          {models.length === 0 ? (
            <span className="mt-1 block font-code-sm text-code-sm text-on-surface-variant">
              No saved models yet — add them in the Models tab.
            </span>
          ) : selectedModelId !== "" ? (
            <span className="mt-1 block font-code-sm text-code-sm text-on-surface-variant">
              {draft.provider}
              {draft.base_url ? ` · ${draft.base_url}` : ""}
            </span>
          ) : (
            <span className="mt-1 block font-code-sm text-code-sm text-on-surface-variant">
              Current model isn’t in the registry — pick one, or add it in the
              Models tab.
            </span>
          )}
        </label>

        {cfg.agent_name === "rain" && (
          <label className="flex items-center gap-2 pt-1">
            <input
              type="checkbox"
              checked={rainDisabledDefault === "1"}
              onChange={async (e) => {
                await setAppSetting.mutateAsync({
                  key: "rain_disabled_default",
                  value: e.target.checked ? "1" : "",
                });
                refetchRainDefault();
              }}
              className="size-4 accent-primary"
            />
            <span className="font-body-md text-body-md text-on-surface">
              Disable Rain by default (new sessions start solo)
            </span>
          </label>
        )}
        {cfg.agent_name === "rain" && setAppSetting.error && (
          <p className="inline-block rounded border border-error/40 bg-error-container/20 px-2 py-1 font-code-sm text-code-sm text-on-error-container">
            Couldn’t save: {setAppSetting.error.message}
          </p>
        )}
      </div>

      {saveError && (
        <p className="mt-4 rounded border border-error/40 bg-error-container/20 px-3 py-2 font-code-sm text-code-sm text-on-error-container">
          Save failed: {saveError}
        </p>
      )}

      {/* Footer: updated-at + reset/save */}
      <div className="mt-4 flex items-center justify-between gap-2 border-t border-outline-variant/30 pt-3">
        <span className="truncate font-code-sm text-code-sm text-on-surface-variant">
          updated {formatTimestamp(cfg.updated_at) || "—"}
        </span>
        <div className="flex shrink-0 gap-2">
          <button
            type="button"
            disabled={!dirty || isSaving}
            onClick={() => {
              setDraft(cfg);
              onDirtyChange(false);
            }}
            className="rounded border border-outline-variant bg-transparent px-3 py-1.5 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:bg-surface-container-high hover:text-on-surface disabled:opacity-50"
          >
            Reset
          </button>
          <button
            type="button"
            disabled={!dirty || isSaving}
            onClick={async () => {
              setSaveError(null);
              try {
                await onSave(draft);
                setSaved(true);
              } catch (e) {
                setSaveError(errorMessage(e));
              }
            }}
            className="inline-flex items-center gap-1.5 rounded border border-primary bg-primary px-3 py-1.5 font-code-sm text-code-sm text-on-primary transition-colors hover:bg-primary-fixed disabled:opacity-50"
          >
            <SaveIcon />
            {isSaving ? "Saving…" : "Save Configuration"}
          </button>
        </div>
      </div>
    </section>
  );
}

// ============================================================================
// Archive — closed sessions (just-closed + archived), newest-closed first
// ============================================================================

function WorktreeKeptBadge({ sessionId }: { sessionId: string }) {
  // C1: queries whether this closed session's isolated worktree still exists on
  // disk (close keeps — never force-removes — a dirty worktree). Only mounted
  // for worktree-backed sessions, so the query runs only where it can matter.
  const { data: keptPath } = useTauriQuery<string | null>(
    "session_worktree_kept",
    { sessionId },
  );
  if (!keptPath) return null;
  return (
    <span
      className="shrink-0 rounded border border-warning/40 bg-warning/15 px-2 py-0.5 font-label-caps text-label-caps text-warning"
      title={`Worktree kept — may have uncommitted work: ${keptPath}`}
    >
      ⚠ Worktree kept
    </span>
  );
}

function ArchivePanel() {
  const { data: sessions = [], isLoading } = useTauriQuery<SessionInfo[]>(
    "list_closed_sessions",
  );
  return (
    <div className="mx-auto h-full max-w-4xl overflow-y-auto overflow-x-hidden px-6 py-6">
      <div className="mb-6">
        <h1 className="font-headline-lg text-headline-lg text-on-surface">
          Archived Sessions
        </h1>
        <p className="mt-1 max-w-prose font-body-md text-body-md text-on-surface-variant">
          Every closed session. Click one to reopen it for review — the duo
          re-spawns via <code>--resume</code>, picking up prior context when
          it's still available (the session view notes it otherwise).
        </p>
      </div>
      {isLoading ? (
        <div className="space-y-2">
          {[0, 1, 2].map((i) => (
            <div
              key={i}
              className="h-14 animate-pulse rounded-lg border border-outline-variant bg-surface-container"
            />
          ))}
        </div>
      ) : sessions.length === 0 ? (
        <p className="rounded-lg border border-outline-variant bg-surface-container px-4 py-6 text-center font-code-sm text-code-sm text-on-surface-variant">
          No closed sessions yet.
        </p>
      ) : (
        <ul className="flex flex-col gap-2">
          {sessions.map((s) => (
            <li key={s.id}>
              <Link
                to={`/sessions/${s.id}`}
                className="flex items-center justify-between gap-3 rounded-lg border border-outline-variant bg-surface-container px-4 py-3 transition-colors hover:border-primary hover:bg-surface-container-high"
              >
                <div className="min-w-0">
                  <p className="truncate font-code-sm text-code-sm text-on-surface">
                    {s.title || "(untitled session)"}
                  </p>
                  <p className="font-code-sm text-code-sm text-on-surface-variant">
                    <code className="text-on-surface-variant">
                      {s.id.slice(0, 8)}
                    </code>
                    <span className="mx-2 text-on-surface-variant/60">·</span>
                    closed {formatTimestamp(s.closed_at ?? "") || "—"}
                  </p>
                </div>
                <div className="flex shrink-0 items-center gap-2">
                  {s.base_repo_path && <WorktreeKeptBadge sessionId={s.id} />}
                  <span
                    className={cn(
                      "shrink-0 rounded border px-2 py-0.5 font-label-caps text-label-caps",
                      s.archived
                        ? "border-outline-variant/40 bg-outline-variant/15 text-on-surface-variant"
                        : "border-tertiary/40 bg-tertiary/15 text-tertiary",
                    )}
                  >
                    {s.archived ? "Archived" : "Closed"}
                  </span>
                </div>
              </Link>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

// ============================================================================
// Updates — check GitHub Releases for a newer bot-hq (check-and-notify)
// ============================================================================

function UpdatesPanel() {
  const { data, isFetching, isError, error, refetch } = useTauriQuery<UpdateInfo>(
    "check_for_update",
    {},
    { retry: false, refetchOnWindowFocus: false, staleTime: 1000 * 60 * 60 },
  );

  return (
    <div className="mx-auto h-full max-w-4xl overflow-y-auto overflow-x-hidden px-6 py-6">
      <div className="mb-6 flex items-start justify-between gap-4">
        <div>
          <h1 className="font-headline-lg text-headline-lg text-on-surface">
            Updates
          </h1>
          <p className="mt-1 max-w-prose font-body-md text-body-md text-on-surface-variant">
            bot-hq checks GitHub Releases for a newer version on launch. The
            install is manual — <span className="text-primary">Download</span>{" "}
            opens the release page in your browser.
          </p>
        </div>
        <button
          type="button"
          onClick={() => refetch()}
          disabled={isFetching}
          className="inline-flex shrink-0 items-center gap-2 rounded border border-primary bg-primary px-3 py-1.5 font-code-sm text-code-sm text-on-primary transition-colors hover:bg-primary-fixed disabled:opacity-50"
        >
          {isFetching ? "Checking…" : "Check now"}
        </button>
      </div>

      <div className="rounded-lg border border-outline-variant bg-surface-container p-5">
        <dl className="flex flex-col gap-2 font-code-sm text-code-sm">
          <div className="flex justify-between gap-4">
            <dt className="text-on-surface-variant">Installed version</dt>
            <dd className="text-on-surface">{data?.current_version ?? "—"}</dd>
          </div>
          <div className="flex justify-between gap-4">
            <dt className="text-on-surface-variant">Latest release</dt>
            <dd className="text-on-surface">
              {isFetching ? "checking…" : data?.latest_version ?? "—"}
            </dd>
          </div>
        </dl>

        <div className="mt-4 border-t border-outline-variant/30 pt-4">
          {isError ? (
            <p className="font-code-sm text-code-sm text-on-surface-variant">
              Couldn&rsquo;t check for updates
              {error?.message ? `: ${error.message}` : ""}. You may be offline,
              or no release has been published yet.
            </p>
          ) : data?.update_available ? (
            <div className="flex items-center justify-between gap-4">
              <p className="font-code-sm text-code-sm text-on-surface">
                A newer version (
                <span className="text-primary">{data.latest_version}</span>) is
                available.
              </p>
              <button
                type="button"
                onClick={() => void openUrl(data.release_url)}
                className="inline-flex shrink-0 items-center rounded border border-primary bg-primary px-3 py-1.5 font-code-sm text-code-sm text-on-primary transition-colors hover:bg-primary-fixed"
              >
                Download
              </button>
            </div>
          ) : data ? (
            <p className="font-code-sm text-code-sm text-on-surface-variant">
              You&rsquo;re on the latest version.
            </p>
          ) : (
            <p className="font-code-sm text-code-sm text-on-surface-variant">
              Checking for updates…
            </p>
          )}
        </div>
      </div>
    </div>
  );
}

// ============================================================================
// Tool Gate — global gated-Bash keywords
// ============================================================================

function ToolGateSection() {
  const { data: keywords = [], refetch, isLoading } =
    useTauriQuery<GatedKeyword[]>("get_tool_gate_keywords");
  const save = useTauriMutation<void, { keywords: GatedKeyword[] }>(
    "set_tool_gate_keywords",
  );

  // The server list is the baseline; `draft` holds in-progress edits. Re-
  // hydrate the draft whenever the server list changes (initial load + after a
  // save's refetch) so dirty-tracking compares against the persisted state.
  const { draft, setDraft, dirty } = useServerDraft<GatedKeyword[]>(keywords);

  const onSave = async () => {
    // Drop blank keywords — they match nothing and only clutter the file.
    await save.mutateAsync({
      keywords: draft.filter((k) => k.keyword.trim() !== ""),
    });
    refetch();
  };

  return (
    <section className="mt-10 border-t border-outline-variant/30 pt-6">
      <div className="mb-4 flex items-start justify-between gap-4">
        <div>
          <h2 className="font-headline-md text-headline-md text-on-surface">
            Gated Bash Keywords
          </h2>
          <p className="mt-1 max-w-prose font-body-md text-body-md text-on-surface-variant">
            One global list for every session. When an agent's Bash command
            contains a keyword, <span className="text-primary">Gate</span> blocks
            it and asks you to Approve/Reject (bot-hq runs it on approve);{" "}
            <span className="text-success">Auto-allow</span> lets it run with
            no prompt. Case-insensitive substring match against the command or
            tool name; commands with no matching keyword run normally.
          </p>
        </div>
        {dirty && (
          <button
            type="button"
            onClick={onSave}
            disabled={save.isPending}
            className="inline-flex shrink-0 items-center gap-2 rounded border border-primary bg-primary px-3 py-1.5 font-code-sm text-code-sm text-on-primary transition-colors hover:bg-primary-fixed disabled:opacity-50"
          >
            <SaveIcon />
            {save.isPending ? "Saving…" : "Save keywords"}
          </button>
        )}
      </div>

      {isLoading ? (
        <div className="h-24 animate-pulse rounded-lg border border-outline-variant bg-surface-container" />
      ) : (
        <div className="rounded-lg border border-outline-variant bg-surface-container p-4">
          <GatedKeywordList
            value={draft}
            onChange={setDraft}
            inputClassName={cn(terminalInputClass, "flex-1")}
            emptyState={
              <p className="py-2 font-code-sm text-code-sm text-on-surface-variant">
                No keywords configured — every Bash command runs ungated. Add
                one (e.g. <code>gh</code>, <code>git push</code>,{" "}
                <code>rm -rf</code>) to gate or auto-allow matching commands.
              </p>
            }
            footer={(addRow) => (
              <div className="mt-3 flex items-center gap-3">
                <Button variant="ghost" size="sm" onClick={addRow}>
                  + Add keyword
                </Button>
                {dirty && (
                  <span className="font-label-caps text-label-caps text-warning">
                    Unsaved changes
                  </span>
                )}
              </div>
            )}
          />
        </div>
      )}
      {save.error && (
        <p className="mt-4 rounded border border-error/40 bg-error-container/20 px-3 py-2 font-code-sm text-code-sm text-on-error-container">
          Save failed: {save.error.message}
        </p>
      )}
    </section>
  );
}

// ============================================================================
// Helpers
// ============================================================================

function roleBorder(name: string): string {
  switch (name) {
    case "brian":
      return "border-primary/60";
    case "rain":
      return "border-outline-variant";
    default:
      return "border-outline-variant";
  }
}

function agentIcon(name: string): ReactElement {
  switch (name) {
    case "brian":
      return <WrenchIcon size={18} className="text-primary" />;
    case "rain":
      return <EyeIcon size={18} className="text-on-surface-variant" />;
    default:
      return <GearIcon size={18} className="text-on-surface-variant" />;
  }
}


