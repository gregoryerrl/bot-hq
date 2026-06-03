import { useCallback, useEffect, useRef, useState } from "react";
import { Link, useBlocker } from "react-router-dom";
import { useTauriQuery, useTauriMutation } from "../hooks/useInvoke";
import { Button } from "../components/ui/Button";
import { cn } from "../lib/cn";
import { parseUtcMs } from "../lib/time";
import { SaveIcon } from "./contextLibraryShared";
import { ClaudeConfigPanel } from "./ClaudeConfig";
import { ModelsPanel } from "./ModelsPanel";
import { PolicyForm } from "../components/PolicyForm";
import type {
  AgentConfigView,
  GatedKeyword,
  GateMode,
  ModelView,
  Policy,
  SessionInfo,
} from "../lib/bindings";

type SettingsSubTab =
  | "agents"
  | "models"
  | "claude"
  | "toolgate"
  | "policy"
  | "archive";

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
  return (
    <div className="flex h-full flex-col bg-background">
      <div className="flex shrink-0 items-center gap-1 border-b border-outline-variant px-4">
        <SubTabButton active={tab === "agents"} onClick={() => setTab("agents")}>
          Agents
        </SubTabButton>
        <SubTabButton active={tab === "models"} onClick={() => setTab("models")}>
          Models
        </SubTabButton>
        <SubTabButton active={tab === "claude"} onClick={() => setTab("claude")}>
          Claude Config
        </SubTabButton>
        <SubTabButton
          active={tab === "toolgate"}
          onClick={() => setTab("toolgate")}
        >
          Tool Gate
        </SubTabButton>
        <SubTabButton active={tab === "policy"} onClick={() => setTab("policy")}>
          Policy
        </SubTabButton>
        <SubTabButton
          active={tab === "archive"}
          onClick={() => setTab("archive")}
        >
          Archive
        </SubTabButton>
      </div>
      <div className="min-h-0 flex-1">
        <div className={cn("h-full", tab !== "agents" && "hidden")}>
          <AgentsPanel />
        </div>
        <div className={cn("h-full", tab !== "models" && "hidden")}>
          <ModelsPanel />
        </div>
        <div className={cn("h-full", tab !== "claude" && "hidden")}>
          <ClaudeConfigPanel />
        </div>
        <div className={cn("h-full", tab !== "toolgate" && "hidden")}>
          <ToolGatePanel />
        </div>
        <div className={cn("h-full", tab !== "policy" && "hidden")}>
          <GlobalPolicyPanel />
        </div>
        <div className={cn("h-full", tab !== "archive" && "hidden")}>
          <ArchivePanel />
        </div>
      </div>
    </div>
  );
}

function SubTabButton({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "border-b-2 px-3 py-2.5 font-code-sm text-code-sm transition-colors",
        active
          ? "border-primary text-primary"
          : "border-transparent text-on-surface-variant hover:text-on-surface",
      )}
    >
      {children}
    </button>
  );
}

function ToolGatePanel() {
  return (
    <div className="mx-auto h-full max-w-7xl overflow-auto px-6 py-6">
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
    await save.mutateAsync({ policy: draft });
    refetch();
  };

  return (
    <div className="mx-auto h-full max-w-4xl overflow-auto px-6 py-6">
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

// Curated provider list for the dropdown. Any provider value not in this
// list flips the dropdown to "Other" and reveals a free-text input so a
// user can name any vendor (or a self-hosted endpoint).
const KNOWN_PROVIDERS = [
  "anthropic",
  "openai",
  "deepseek",
  "local",
] as const;

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
    <div className="mx-auto h-full max-w-7xl overflow-auto px-6 py-6">
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
        <div className="grid grid-cols-1 gap-gutter xl:grid-cols-3">
          {[0, 1, 2].map((i) => (
            <div
              key={i}
              className="h-64 animate-pulse rounded-lg border border-outline-variant bg-surface-container"
            />
          ))}
        </div>
      ) : (
        <div className="grid grid-cols-1 gap-gutter xl:grid-cols-3">
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
    </div>
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
  const [tokenVisible, setTokenVisible] = useState(false);
  const [saved, setSaved] = useState(false);
  // Force the custom (free-text) view even when the draft happens to match a
  // saved model — set when the user explicitly picks "Custom…".
  const [customMode, setCustomMode] = useState(false);
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

  const isEmma = cfg.agent_name === "emma";
  // Which saved model (if any) the current config corresponds to. Exact match
  // on the spawn-relevant fields so the dropdown reflects the agent's model.
  const selectedModelId =
    models.find(
      (m) =>
        m.provider === draft.provider &&
        m.model_name === draft.model_name &&
        (m.base_url ?? "") === (draft.base_url ?? ""),
    )?.id ?? "";
  // Show free-text detail fields for Emma always, or when no saved model
  // matches / the user chose Custom.
  const showCustom = isEmma || customMode || selectedModelId === "";

  // Save-all fan-out: parent increments saveAllSignal; each dirty row
  // triggers its own save. Skipping initial mount via a ref guards against
  // saving on first render when saveAllSignal=0.
  const lastSeenSignal = useRef(saveAllSignal);
  useEffect(() => {
    if (saveAllSignal === lastSeenSignal.current) return;
    lastSeenSignal.current = saveAllSignal;
    if (!dirty) return;
    onSave(draft).then(() => setSaved(true));
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
    if (dirty) setSaved(false);
  }

  const providerIsCustom = !KNOWN_PROVIDERS.includes(
    draft.provider as (typeof KNOWN_PROVIDERS)[number],
  );

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
          <span className="text-lg leading-none" aria-hidden>
            {agentIcon(cfg.agent_name)}
          </span>
          <h2 className="truncate font-headline-md text-headline-md capitalize text-on-surface">
            {cfg.agent_name}
          </h2>
          {dirty && (
            <span className="shrink-0 rounded border border-amber-500/40 bg-amber-500/15 px-1.5 py-0.5 font-label-caps text-label-caps text-amber-300">
              Unsaved
            </span>
          )}
          {saved && !dirty && (
            <span className="shrink-0 rounded border border-emerald-500/40 bg-emerald-500/15 px-1.5 py-0.5 font-label-caps text-label-caps text-emerald-300">
              Saved ✓
            </span>
          )}
        </div>
        <RoleChip agentName={cfg.agent_name} />
      </div>

      {/* Form fields */}
      <div className="flex flex-1 flex-col gap-4">
        {/* Brian + Rain pick from saved models; their choice IS their default
            model for new sessions. Emma keeps free-text (out of scope). */}
        {!isEmma && (
          <label className="block">
            <FieldLabel>Model</FieldLabel>
            <select
              value={showCustom ? "__custom__" : selectedModelId}
              onChange={(e) => {
                const v = e.target.value;
                if (v === "__custom__") {
                  setCustomMode(true);
                  return;
                }
                const m = models.find((x) => x.id === v);
                if (m) {
                  setCustomMode(false);
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
              {models.map((m) => (
                <option key={m.id} value={m.id}>
                  {m.display_name}
                  {m.model_name ? ` — ${m.model_name}` : ""}
                </option>
              ))}
              <option value="__custom__">Custom…</option>
            </select>
            {models.length === 0 ? (
              <span className="mt-1 block font-code-sm text-code-sm text-on-surface-variant">
                No saved models yet — add them in the Models tab.
              </span>
            ) : (
              !showCustom && (
                <span className="mt-1 block font-code-sm text-code-sm text-on-surface-variant">
                  {draft.provider}
                  {draft.base_url ? ` · ${draft.base_url}` : ""}
                </span>
              )
            )}
          </label>
        )}

        {showCustom && (
          <>
            <label className="block">
              <FieldLabel>Provider</FieldLabel>
              <select
                value={providerIsCustom ? "other" : draft.provider}
                onChange={(e) => {
                  if (e.target.value === "other") {
                    setDraft({ ...draft, provider: "" });
                  } else {
                    setDraft({ ...draft, provider: e.target.value });
                  }
                }}
                className="w-full rounded border border-outline-variant bg-surface-container-lowest px-2 py-1.5 font-code-sm text-code-sm text-on-surface focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary"
              >
                <option value="anthropic">Anthropic</option>
                <option value="openai">OpenAI</option>
                <option value="deepseek">DeepSeek</option>
                <option value="local">Local (llama.cpp)</option>
                <option value="other">Other (custom)</option>
              </select>
              {providerIsCustom && (
                <input
                  type="text"
                  value={draft.provider}
                  onChange={(e) =>
                    setDraft({ ...draft, provider: e.target.value })
                  }
                  placeholder="Custom provider"
                  className={cn("mt-2", terminalInputClass)}
                />
              )}
            </label>

            <label className="block">
              <FieldLabel>Model Name</FieldLabel>
              <input
                type="text"
                value={draft.model_name}
                onChange={(e) =>
                  setDraft({ ...draft, model_name: e.target.value })
                }
                placeholder="claude-opus-4-7"
                className={terminalInputClass}
              />
            </label>

            <label className="block">
              <FieldLabel>Base URL</FieldLabel>
              <input
                type="text"
                value={draft.base_url ?? ""}
                onChange={(e) =>
                  setDraft({ ...draft, base_url: e.target.value || null })
                }
                placeholder="(provider default)"
                className={terminalInputClass}
              />
            </label>

            <label className="block">
              <FieldLabel>Auth Token</FieldLabel>
              <div className="relative">
                <input
                  type={tokenVisible ? "text" : "password"}
                  value={draft.auth_token ?? ""}
                  onChange={(e) =>
                    setDraft({ ...draft, auth_token: e.target.value || null })
                  }
                  placeholder="(unset — uses provider env vars)"
                  className={cn(terminalInputClass, "pr-12")}
                />
                <button
                  type="button"
                  onClick={() => setTokenVisible((v) => !v)}
                  className="absolute inset-y-0 right-0 px-2 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:text-on-surface"
                >
                  {tokenVisible ? "Hide" : "Show"}
                </button>
              </div>
            </label>
          </>
        )}

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
      </div>

      {/* Footer: updated-at + reset/save */}
      <div className="mt-4 flex items-center justify-between gap-2 border-t border-outline-variant/30 pt-3">
        <span className="truncate font-code-sm text-code-sm text-on-surface-variant">
          updated {formatTimestamp(cfg.updated_at)}
        </span>
        <div className="flex shrink-0 gap-2">
          <button
            type="button"
            disabled={!dirty || isSaving}
            onClick={() => {
              setDraft(cfg);
              setTokenVisible(false);
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
              await onSave(draft);
              setSaved(true);
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

function ArchivePanel() {
  const { data: sessions = [], isLoading } = useTauriQuery<SessionInfo[]>(
    "list_closed_sessions",
  );
  return (
    <div className="mx-auto h-full max-w-4xl overflow-auto px-6 py-6">
      <div className="mb-6">
        <h1 className="font-headline-lg text-headline-lg text-on-surface">
          Archived Sessions
        </h1>
        <p className="mt-1 max-w-prose font-body-md text-body-md text-on-surface-variant">
          Every closed session. Click one to reopen it for review — the duo
          re-spawns via <code>--resume</code>, though resume can fail for
          sessions idle more than a few minutes (the session view shows the
          error if so).
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
                    closed {formatTimestamp(s.closed_at ?? "")}
                  </p>
                </div>
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
              </Link>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

// ============================================================================
// Tool Gate — global gated-Bash keywords
// ============================================================================

const GATE_MODES: GateMode[] = ["gate", "auto_allow"];

function ToolGateSection() {
  const { data: keywords = [], refetch, isLoading } =
    useTauriQuery<GatedKeyword[]>("get_tool_gate_keywords");
  const save = useTauriMutation<void, { keywords: GatedKeyword[] }>(
    "set_tool_gate_keywords",
  );

  // The server list is the baseline; `draft` holds in-progress edits. Re-
  // hydrate the draft whenever the server list changes (initial load + after a
  // save's refetch) so dirty-tracking compares against the persisted state.
  const serverJson = JSON.stringify(keywords);
  const [draft, setDraft] = useState<GatedKeyword[]>(keywords);
  const lastServer = useRef(serverJson);
  useEffect(() => {
    if (lastServer.current !== serverJson) {
      lastServer.current = serverJson;
      setDraft(keywords);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [serverJson]);

  const dirty = JSON.stringify(draft) !== serverJson;

  const updateRow = (i: number, patch: Partial<GatedKeyword>) =>
    setDraft((d) => d.map((k, idx) => (idx === i ? { ...k, ...patch } : k)));
  const removeRow = (i: number) =>
    setDraft((d) => d.filter((_, idx) => idx !== i));
  const addRow = () =>
    setDraft((d) => [...d, { keyword: "", mode: "gate" }]);

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
            <span className="text-emerald-300">Auto-allow</span> lets it run with
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
          {draft.length === 0 ? (
            <p className="py-2 font-code-sm text-code-sm text-on-surface-variant">
              No keywords configured — every Bash command runs ungated. Add one
              (e.g. <code>gh</code>, <code>git push</code>, <code>rm -rf</code>)
              to gate or auto-allow matching commands.
            </p>
          ) : (
            <ul className="flex flex-col gap-2">
              {draft.map((k, i) => (
                <li key={i} className="flex items-center gap-2">
                  <input
                    type="text"
                    value={k.keyword}
                    onChange={(e) => updateRow(i, { keyword: e.target.value })}
                    placeholder="keyword (e.g. gh issue, git push, curl)"
                    className={cn(terminalInputClass, "flex-1")}
                  />
                  <div className="flex shrink-0 overflow-hidden rounded border border-outline-variant">
                    {GATE_MODES.map((m) => {
                      const active = k.mode === m;
                      const activeCls =
                        m === "gate"
                          ? "bg-primary/15 text-primary"
                          : "bg-emerald-500/15 text-emerald-300";
                      return (
                        <button
                          key={m}
                          type="button"
                          onClick={() => updateRow(i, { mode: m })}
                          className={cn(
                            "px-2.5 py-1 font-label-caps text-label-caps transition-colors",
                            active
                              ? activeCls
                              : "bg-transparent text-on-surface-variant hover:text-on-surface",
                          )}
                        >
                          {m === "gate" ? "Gate" : "Auto-allow"}
                        </button>
                      );
                    })}
                  </div>
                  <button
                    type="button"
                    onClick={() => removeRow(i)}
                    aria-label="Remove keyword"
                    className="shrink-0 rounded border border-outline-variant bg-transparent px-2 py-1 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:bg-surface-container-high hover:text-on-surface"
                  >
                    ✕
                  </button>
                </li>
              ))}
            </ul>
          )}
          <div className="mt-3 flex items-center gap-3">
            <Button variant="ghost" size="sm" onClick={addRow}>
              + Add keyword
            </Button>
            {dirty && (
              <span className="font-label-caps text-label-caps text-amber-300">
                Unsaved changes
              </span>
            )}
          </div>
        </div>
      )}
    </section>
  );
}

// ============================================================================
// Helpers
// ============================================================================

const terminalInputClass = cn(
  "w-full border-0 border-b border-outline-variant bg-transparent",
  "rounded-none px-0 py-1.5 font-code-sm text-code-sm text-on-surface",
  "placeholder:text-on-surface-variant caret-primary",
  "focus:border-primary focus:outline-none",
);

function FieldLabel({ children }: { children: React.ReactNode }) {
  return (
    <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
      {children}
    </span>
  );
}

function RoleChip({ agentName }: { agentName: string }) {
  const { label, bg, text, border } = roleStyle(agentName);
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center rounded border px-2 py-0.5 font-label-caps text-label-caps",
        bg,
        text,
        border,
      )}
    >
      {label}
    </span>
  );
}

function roleStyle(name: string): {
  label: string;
  bg: string;
  text: string;
  border: string;
} {
  switch (name) {
    case "brian":
      return {
        label: "ACTIVE",
        bg: "bg-primary/15",
        text: "text-primary",
        border: "border-primary/30",
      };
    case "rain":
      return {
        label: "STANDBY",
        bg: "bg-outline-variant/15",
        text: "text-on-surface-variant",
        border: "border-outline-variant/30",
      };
    case "emma":
      return {
        label: "PRIMARY",
        bg: "bg-secondary/15",
        text: "text-secondary",
        border: "border-secondary/30",
      };
    default:
      return {
        label: name.toUpperCase(),
        bg: "bg-outline-variant/15",
        text: "text-on-surface-variant",
        border: "border-outline-variant/30",
      };
  }
}

function roleBorder(name: string): string {
  switch (name) {
    case "brian":
      return "border-primary/60";
    case "emma":
      return "border-secondary/60";
    case "rain":
      return "border-outline-variant";
    default:
      return "border-outline-variant";
  }
}

function agentIcon(name: string): string {
  switch (name) {
    case "brian":
      return "👷";
    case "rain":
      return "💧";
    case "emma":
      return "🤖";
    default:
      return "⚙️";
  }
}

function formatTimestamp(iso: string): string {
  if (!iso) return "—";
  // Zone-safe: legacy rows can be zone-less; treat them as UTC.
  const ms = parseUtcMs(iso);
  if (!Number.isFinite(ms)) return iso;
  return new Date(ms).toLocaleString();
}

