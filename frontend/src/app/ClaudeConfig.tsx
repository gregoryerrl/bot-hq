import { useEffect, useRef, useState, type ReactElement } from "react";
import { useTauriQuery, useTauriMutation, errorMessage } from "../hooks/useInvoke";
import { Button } from "../components/ui/Button";
import { cn } from "../lib/cn";
import { SaveIcon } from "./contextLibraryShared";
import {
  GearIcon,
  OverviewIcon,
  SkillsIcon,
  PluginsIcon,
  McpIcon,
  MemoryIcon,
  PermissionsIcon,
  type IconProps,
} from "../components/icons";
import type {
  AgentOverride,
  ClaudeConfigView,
  ClaudeOverrides,
  Inheritance,
  McpServerItem,
  PluginItem,
  SessionInfo,
  SettingItem,
  SkillItem,
  SkillVisibility,
} from "../lib/bindings";

// ============================================================================
// Claude Config — the Settings subtab that surfaces the Claude Code config the
// bot-hq agents inherit, with a per-agent override layer.
//
// 2-pane: left = surface nav (reuses the Context Library shell's visual idiom);
// right = structured detail with an INHERITANCE LENS (which agents pick up the
// surface) + override controls that write `<data_dir>/claude-overrides.json`.
// Global ~/.claude editing (Phase 3) is read-only here for now; the override
// layer (agent-scoped) is fully wired.
// ============================================================================

type SurfaceId =
  | "overview"
  | "core"
  | "skills"
  | "plugins"
  | "mcp"
  | "memory"
  | "permissions";

const SURFACES: {
  id: SurfaceId;
  label: string;
  Icon: (props: IconProps) => ReactElement;
}[] = [
  { id: "overview", label: "Overview", Icon: OverviewIcon },
  { id: "core", label: "Core knobs", Icon: GearIcon },
  { id: "skills", label: "Skills", Icon: SkillsIcon },
  { id: "plugins", label: "Plugins", Icon: PluginsIcon },
  { id: "mcp", label: "MCP servers", Icon: McpIcon },
  { id: "memory", label: "Memory & instructions", Icon: MemoryIcon },
  { id: "permissions", label: "Permissions", Icon: PermissionsIcon },
];

/** Empty override → the "no overrides" baseline (everything inherited). */
function emptyAll(): AgentOverride {
  return {};
}

export function ClaudeConfigPanel() {
  const {
    data: config,
    isLoading: configLoading,
    refetch: refetchConfig,
  } = useTauriQuery<ClaudeConfigView>("claude_config_read");
  const { data: serverOverrides, refetch: refetchOverrides } =
    useTauriQuery<ClaudeOverrides>("get_claude_overrides");
  const save = useTauriMutation<void, { overrides: ClaudeOverrides }>(
    "set_claude_overrides",
  );

  // Global edits to the real ~/.claude/settings.json (apply immediately to your
  // own Claude AND the agents that inherit it).
  const setString = useTauriMutation<void, { key: string; value: string | null }>(
    "claude_config_set_string",
  );
  const setBool = useTauriMutation<void, { key: string; value: boolean | null }>(
    "claude_config_set_bool",
  );
  const setPluginEnabled = useTauriMutation<
    void,
    { key: string; enabled: boolean | null }
  >("claude_config_set_plugin_enabled");

  // GLOBAL settings.json edits are STAGED (not written until Save), so the
  // whole panel uses one "edit then Save" model alongside the overrides draft.
  const [pendingStrings, setPendingStrings] = useState<Record<string, string | null>>({});
  const [pendingBools, setPendingBools] = useState<Record<string, boolean>>({});
  const [pendingPlugins, setPendingPlugins] = useState<Record<string, boolean>>({});
  const stageString = (key: string, value: string | null) =>
    setPendingStrings((p) => ({ ...p, [key]: value }));
  const stageBool = (key: string, value: boolean) =>
    setPendingBools((p) => ({ ...p, [key]: value }));
  const stagePlugin = (key: string, enabled: boolean) =>
    setPendingPlugins((p) => ({ ...p, [key]: enabled }));
  const clearPending = () => {
    setPendingStrings({});
    setPendingBools({});
    setPendingPlugins({});
  };
  const pendingCount =
    Object.keys(pendingStrings).length +
    Object.keys(pendingBools).length +
    Object.keys(pendingPlugins).length;

  // After a save, offer to restart running agents so they pick up the new
  // config (overrides + inherited settings are read at agent SPAWN, so a live
  // session keeps its old config until respawned).
  const [showRestart, setShowRestart] = useState(false);
  const [restartError, setRestartError] = useState<string | null>(null);
  const { data: sessions } = useTauriQuery<SessionInfo[]>("list_sessions");
  const respawn = useTauriMutation<void, { sessionId: string }>("restart_session");
  const activeSessions = (sessions ?? []).filter(
    (s) => !s.archived && !s.closed_at,
  );
  const onRestartAgents = async () => {
    setRestartError(null);
    try {
      for (const s of activeSessions) {
        await respawn.mutateAsync({ sessionId: s.id });
      }
      setShowRestart(false);
    } catch (e) {
      // A failed respawn aborts the loop — surface it instead of leaving the
      // banner silently stuck (some sessions may already have restarted).
      setRestartError(errorMessage(e));
    }
  };

  const [surface, setSurface] = useState<SurfaceId>("overview");

  // Draft override store, re-hydrated whenever the server copy changes
  // (initial load + after a save refetch) — same pattern as ToolGateSection.
  const serverJson = JSON.stringify(serverOverrides ?? {});
  const [draft, setDraft] = useState<ClaudeOverrides>(serverOverrides ?? {});
  const lastServer = useRef(serverJson);
  useEffect(() => {
    if (lastServer.current !== serverJson) {
      lastServer.current = serverJson;
      setDraft(serverOverrides ?? {});
    }
  }, [serverJson, serverOverrides]);

  const overridesDirty = JSON.stringify(draft) !== serverJson;
  const dirty = overridesDirty || pendingCount > 0;
  const all: AgentOverride = draft._all ?? emptyAll();
  const brian: AgentOverride = draft.brian ?? emptyAll();
  const rain: AgentOverride = draft.rain ?? emptyAll();

  // ---- override mutators (all write the `_all` fan-out: applies to every
  // agent; Rain ignores skill/plugin entries under --bare). ----
  const patchAll = (patch: Partial<AgentOverride>) =>
    setDraft((d) => ({ ...d, _all: { ...(d._all ?? {}), ...patch } }));
  // Per-agent effort/ultracode overrides (layered over `_all` at resolve time).
  const patchBrian = (patch: Partial<AgentOverride>) =>
    setDraft((d) => ({ ...d, brian: { ...(d.brian ?? {}), ...patch } }));
  const patchRain = (patch: Partial<AgentOverride>) =>
    setDraft((d) => ({ ...d, rain: { ...(d.rain ?? {}), ...patch } }));

  const setSkill = (name: string, vis: SkillVisibility | null) =>
    setDraft((d) => {
      const skills = { ...((d._all ?? {}).skills ?? {}) };
      if (vis === null) delete skills[name];
      else skills[name] = vis;
      return { ...d, _all: { ...(d._all ?? {}), skills } };
    });

  const setPlugin = (key: string, enabled: boolean | null) =>
    setDraft((d) => {
      const plugins = { ...((d._all ?? {}).plugins ?? {}) };
      if (enabled === null) delete plugins[key];
      else plugins[key] = enabled;
      return { ...d, _all: { ...(d._all ?? {}), plugins } };
    });

  const setMcp = (name: string, forward: boolean | null) =>
    setDraft((d) => {
      const mcp = { ...((d._all ?? {}).mcp ?? {}) };
      if (forward === null) delete mcp[name];
      else mcp[name] = forward;
      return { ...d, _all: { ...(d._all ?? {}), mcp } };
    });

  const saving =
    save.isPending ||
    setString.isPending ||
    setBool.isPending ||
    setPluginEnabled.isPending;

  const onSave = async () => {
    // Flush staged global settings.json edits, then per-agent overrides.
    for (const [key, value] of Object.entries(pendingStrings)) {
      await setString.mutateAsync({ key, value });
    }
    for (const [key, value] of Object.entries(pendingBools)) {
      await setBool.mutateAsync({ key, value });
    }
    for (const [key, enabled] of Object.entries(pendingPlugins)) {
      await setPluginEnabled.mutateAsync({ key, enabled });
    }
    if (overridesDirty) {
      await save.mutateAsync({ overrides: draft });
    }
    clearPending();
    refetchConfig();
    refetchOverrides();
    setShowRestart(true);
  };
  const onReset = () => {
    clearPending();
    setDraft(serverOverrides ?? {});
  };

  if (configLoading || !config) {
    return (
      <div className="flex h-full items-center justify-center">
        <div className="h-32 w-2/3 animate-pulse rounded-lg border border-outline-variant bg-surface-container" />
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col bg-background">
      {/* Unsaved-changes bar (takes priority over the post-save restart banner). */}
      {dirty ? (
        <div className="flex items-center justify-between gap-3 border-b border-primary/30 bg-primary/10 px-4 py-2">
          <span className="font-code-sm text-code-sm text-on-surface">
            Unsaved changes — global edits write your settings.json; agent
            overrides take effect after restarting agents.
          </span>
          <div className="flex gap-2">
            <Button variant="ghost" size="sm" onClick={onReset}>
              Reset
            </Button>
            <button
              type="button"
              onClick={onSave}
              disabled={saving}
              className="inline-flex items-center gap-2 rounded border border-primary bg-primary px-3 py-1.5 font-code-sm text-code-sm text-on-primary hover:bg-primary-fixed disabled:opacity-50"
            >
              <SaveIcon />
              {saving ? "Saving…" : "Save changes"}
            </button>
          </div>
        </div>
      ) : showRestart ? (
        <div className="flex items-center justify-between gap-3 border-b border-emerald-500/30 bg-emerald-500/10 px-4 py-2">
          <span className="font-code-sm text-code-sm text-on-surface">
            Saved ✓{" "}
            {activeSessions.length > 0
              ? `${activeSessions.length} agent session(s) are running — restart them to apply the new config now.`
              : "Applies to new agent sessions. Restart Claude Code itself for restart-only settings (model, thinking)."}
            {restartError && (
              <span className="ml-2 text-on-error-container">
                — restart failed: {restartError}
              </span>
            )}
          </span>
          <div className="flex gap-2">
            {activeSessions.length > 0 && (
              <button
                type="button"
                onClick={onRestartAgents}
                disabled={respawn.isPending}
                className="inline-flex items-center gap-2 rounded border border-emerald-500/50 bg-emerald-500/15 px-3 py-1.5 font-code-sm text-code-sm text-emerald-200 hover:bg-emerald-500/25 disabled:opacity-50"
              >
                {respawn.isPending
                  ? "Restarting…"
                  : `Restart ${activeSessions.length} agent session(s)`}
              </button>
            )}
            <Button variant="ghost" size="sm" onClick={() => setShowRestart(false)}>
              Dismiss
            </Button>
          </div>
        </div>
      ) : null}

      <div className="flex min-h-0 flex-1">
        {/* Left surface nav */}
        <nav className="w-60 shrink-0 overflow-auto border-r border-outline-variant bg-surface-container-low p-2">
          <ConfigDirHeader config={config} />
          <ul className="mt-2 flex flex-col gap-0.5">
            {SURFACES.map((s) => {
              const count = surfaceCount(s.id, config);
              return (
                <li key={s.id}>
                  <button
                    type="button"
                    onClick={() => setSurface(s.id)}
                    className={cn(
                      "flex w-full items-center justify-between rounded px-2.5 py-1.5 text-left font-code-sm text-code-sm transition-colors",
                      surface === s.id
                        ? "bg-primary/15 text-primary"
                        : "text-on-surface-variant hover:bg-surface-container-high hover:text-on-surface",
                    )}
                  >
                    <span className="flex items-center gap-2">
                      <s.Icon size={14} />
                      {s.label}
                    </span>
                    {count !== null && (
                      <span className="rounded bg-surface-container-highest px-1.5 font-label-caps text-label-caps text-on-surface-variant">
                        {count}
                      </span>
                    )}
                  </button>
                </li>
              );
            })}
          </ul>
        </nav>

        {/* Right detail pane */}
        <div className="min-w-0 flex-1 overflow-auto p-6">
          {surface === "overview" && <OverviewPane config={config} />}
          {surface === "core" && (
            <CorePane
              config={config}
              all={all}
              brian={brian}
              rain={rain}
              patchBrian={patchBrian}
              patchRain={patchRain}
              pendingStrings={pendingStrings}
              pendingBools={pendingBools}
              stageString={stageString}
              stageBool={stageBool}
            />
          )}
          {surface === "skills" && (
            <SkillsPane config={config} all={all} setSkill={setSkill} />
          )}
          {surface === "plugins" && (
            <PluginsPane
              config={config}
              all={all}
              setPlugin={setPlugin}
              pendingPlugins={pendingPlugins}
              stagePlugin={stagePlugin}
            />
          )}
          {surface === "mcp" && (
            <McpPane config={config} all={all} setMcp={setMcp} />
          )}
          {surface === "memory" && (
            <MemoryPane config={config} all={all} patchAll={patchAll} />
          )}
          {surface === "permissions" && <PermissionsPane config={config} />}
        </div>
      </div>
    </div>
  );
}

function surfaceCount(id: SurfaceId, c: ClaudeConfigView): number | null {
  switch (id) {
    case "skills":
      return c.skills.length;
    case "plugins":
      return c.plugins.length;
    case "mcp":
      return c.mcp_servers.length;
    case "core":
      return c.core_knobs.length;
    case "overview":
      return c.warnings.length || null;
    default:
      return null;
  }
}

// ---------------------------------------------------------------------------
// Shared bits
// ---------------------------------------------------------------------------

function ConfigDirHeader({ config }: { config: ClaudeConfigView }) {
  return (
    <div className="rounded border border-outline-variant bg-surface-container p-2">
      <div className="font-label-caps text-label-caps text-on-surface-variant">
        Config dir
      </div>
      <div className="mt-0.5 truncate font-code-sm text-code-sm text-on-surface" title={config.config_dir}>
        {config.config_dir}
      </div>
      <div className="mt-0.5 font-code-sm text-code-sm text-on-surface-variant">
        {config.config_dir_source}
      </div>
      {config.managed_settings_present && (
        <div className="mt-1 rounded border border-amber-500/40 bg-amber-500/15 px-1.5 py-0.5 font-label-caps text-label-caps text-amber-300">
          Managed policy active
        </div>
      )}
    </div>
  );
}

/** Inheritance lens: which agents pick this surface up, which don't. */
function InheritanceBadges({ inheritance }: { inheritance: Inheritance }) {
  return (
    <div className="flex flex-wrap items-center gap-1" title={inheritance.note}>
      {inheritance.inherited_by.map((a) => (
        <span
          key={`in-${a}`}
          className="rounded border border-emerald-500/40 bg-emerald-500/10 px-1.5 py-0.5 font-label-caps text-label-caps capitalize text-emerald-300"
        >
          {a} inherits
        </span>
      ))}
      {inheritance.skipped_by.map((a) => (
        <span
          key={`sk-${a}`}
          className="rounded border border-outline-variant/50 bg-surface-container px-1.5 py-0.5 font-label-caps text-label-caps capitalize text-on-surface-variant"
        >
          {a} skips
        </span>
      ))}
      {inheritance.inherited_by.length === 0 &&
        inheritance.skipped_by.length === 0 && (
          <span className="font-label-caps text-label-caps text-on-surface-variant">
            n/a
          </span>
        )}
    </div>
  );
}

function PaneHeader({
  title,
  blurb,
}: {
  title: string;
  blurb: string;
}) {
  return (
    <div className="mb-4">
      <h2 className="font-headline-md text-headline-md text-on-surface">
        {title}
      </h2>
      <p className="mt-1 max-w-prose font-body-md text-body-md text-on-surface-variant">
        {blurb}
      </p>
    </div>
  );
}

const selectClass = cn(
  "rounded border border-outline-variant bg-surface-container-lowest px-2 py-1",
  "font-code-sm text-code-sm text-on-surface focus:border-primary focus:outline-none",
);

// ---------------------------------------------------------------------------
// Panes
// ---------------------------------------------------------------------------

function OverviewPane({ config }: { config: ClaudeConfigView }) {
  return (
    <div>
      <PaneHeader
        title="What your agents inherit"
        blurb="bot-hq's agents are claude-code subprocesses, so your ~/.claude config flows into them. This tab shows what each agent picks up and lets you override it per-agent without touching your own Claude."
      />
      <dl className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <Stat label="Config dir" value={config.config_dir} mono />
        <Stat label="Source" value={config.config_dir_source} />
        <Stat
          label="~/.claude.json"
          value={config.home_claude_json.present ? "present (loads MCP)" : "absent"}
        />
        <Stat label="Skills" value={`${config.skills.length}`} />
        <Stat label="Plugins" value={`${config.plugins.length}`} />
        <Stat label="MCP servers" value={`${config.mcp_servers.length}`} />
        <Stat
          label="Managed policy"
          value={config.managed_settings_present ? "ACTIVE (overrides bot-hq)" : "none"}
        />
        <Stat
          label="Memory projects"
          value={`${config.memory.projects_with_memory}`}
        />
      </dl>
      {config.warnings.length > 0 && (
        <div className="mt-5 rounded-lg border border-amber-500/40 bg-amber-500/10 p-3">
          <div className="mb-1 font-label-caps text-label-caps text-amber-300">
            Notices
          </div>
          <ul className="list-inside list-disc font-code-sm text-code-sm text-on-surface-variant">
            {config.warnings.map((w, i) => (
              <li key={i}>{w}</li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

function Stat({
  label,
  value,
  mono,
}: {
  label: string;
  value: string;
  mono?: boolean;
}) {
  return (
    <div className="rounded border border-outline-variant bg-surface-container p-2.5">
      <dt className="font-label-caps text-label-caps text-on-surface-variant">
        {label}
      </dt>
      <dd
        className={cn(
          "mt-0.5 truncate text-on-surface",
          mono ? "font-code-sm text-code-sm" : "font-body-md text-body-md",
        )}
        title={value}
      >
        {value}
      </dd>
    </div>
  );
}

function Row({
  children,
  active,
}: {
  children: React.ReactNode;
  active?: boolean;
}) {
  return (
    <li
      className={cn(
        "flex flex-col gap-2 rounded-lg border bg-surface-container p-3",
        active ? "border-primary/50" : "border-outline-variant",
      )}
    >
      {children}
    </li>
  );
}

/** Effort levels offered to agents + the global env-routed knob. `max` is
 *  session-only in claude-code and only persists via CLAUDE_CODE_EFFORT_LEVEL. */
const EFFORT_OPTS = ["low", "medium", "high", "xhigh", "max"];

/** Lightweight schema registry: how to edit each global core knob. */
const KNOB_EDITORS: Record<
  string,
  { type: "enum" | "bool" | "text"; options?: string[] }
> = {
  // Effort routes through the env var (the only persistent lever that accepts
  // `max`); the writer drops the legacy `effortLevel` field on change.
  "env.CLAUDE_CODE_EFFORT_LEVEL": { type: "enum", options: EFFORT_OPTS },
  model: { type: "text" },
  editorMode: { type: "enum", options: ["normal", "vim"] },
  alwaysThinkingEnabled: { type: "bool" },
  voiceEnabled: { type: "bool" },
  "env.CLAUDE_CODE_MAX_OUTPUT_TOKENS": { type: "text" },
};

/** Per-knob explanatory note rendered under the row (optional). */
const KNOB_NOTES: Record<string, string> = {
  "env.CLAUDE_CODE_EFFORT_LEVEL":
    "max is session-only in claude-code; bot-hq persists it via the CLAUDE_CODE_EFFORT_LEVEL env var (which overrides effortLevel).",
};

function CorePane({
  config,
  all,
  brian,
  rain,
  patchBrian,
  patchRain,
  pendingStrings,
  pendingBools,
  stageString,
  stageBool,
}: {
  config: ClaudeConfigView;
  all: AgentOverride;
  brian: AgentOverride;
  rain: AgentOverride;
  patchBrian: (p: Partial<AgentOverride>) => void;
  patchRain: (p: Partial<AgentOverride>) => void;
  pendingStrings: Record<string, string | null>;
  pendingBools: Record<string, boolean>;
  stageString: (key: string, value: string | null) => void;
  stageBool: (key: string, value: boolean) => void;
}) {
  return (
    <div>
      <PaneHeader
        title="Core knobs"
        blurb="Edit your settings.json (applies to your own Claude AND the agents that inherit it) — staged until you Save. The agent runtime overrides below apply on the next agent spawn only."
      />
      <ul className="flex flex-col gap-2">
        {config.core_knobs.map((k: SettingItem) => {
          const staged = k.key in pendingStrings;
          const stagedBool = k.key in pendingBools;
          return (
            <Row key={k.key} active={staged || stagedBool}>
              <div className="flex items-center justify-between gap-3">
                <div className="min-w-0">
                  <div className="font-code-sm text-code-sm text-on-surface">
                    {k.label}
                  </div>
                  <div className="truncate font-label-caps text-label-caps text-on-surface-variant">
                    {k.key} · {k.source}
                  </div>
                </div>
                <KnobEditor
                  knob={k}
                  pendingStrings={pendingStrings}
                  pendingBools={pendingBools}
                  stageString={stageString}
                  stageBool={stageBool}
                />
              </div>
              <InheritanceBadges inheritance={k.inheritance} />
              {KNOB_NOTES[k.key] && (
                <p className="font-label-caps text-label-caps text-on-surface-variant">
                  {KNOB_NOTES[k.key]}
                </p>
              )}
            </Row>
          );
        })}
      </ul>

      <h3 className="mb-2 mt-6 font-headline-sm text-headline-sm text-on-surface">
        Agent runtime overrides
      </h3>
      <p className="mb-2 max-w-prose font-body-md text-body-md text-on-surface-variant">
        Per-agent effort & ultracode, applied on the next agent spawn. Set each
        agent independently so a deep-reasoning effort isn&apos;t pushed blindly
        onto a non-Anthropic model.
      </p>
      <div className="flex flex-col gap-2">
        <AgentEffortOverride
          title="Brian"
          roleLabel="HANDS"
          ov={brian}
          patch={patchBrian}
          inheritedEffort={all.effort}
          isEyes={false}
        />
        <AgentEffortOverride
          title="Rain"
          roleLabel="EYES"
          ov={rain}
          patch={patchRain}
          inheritedEffort={all.effort}
          isEyes={true}
        />
      </div>
    </div>
  );
}

/** One agent's effort + ultracode override. Narrow write-coupling keeps `max`
 *  and ultracode mutually exclusive while preserving the valid `xhigh`+ultracode
 *  pair (ultracode IS xhigh + dynamic workflows). */
function AgentEffortOverride({
  title,
  roleLabel,
  ov,
  patch,
  inheritedEffort,
  isEyes,
}: {
  title: string;
  roleLabel: string;
  ov: AgentOverride;
  patch: (p: Partial<AgentOverride>) => void;
  inheritedEffort?: string | null;
  isEyes: boolean;
}) {
  const ultracodeOn = ov.ultracode === true;
  const effortMax = ov.effort === "max";
  // Conflict-aware disabling: never disable BOTH at once, so a pre-existing
  // (e.g. legacy) override that set max + ultracode together stays escapable.
  const effortDisabled = ultracodeOn && !effortMax;
  const ultracodeDisabled = isEyes || (effortMax && !ultracodeOn);
  const onEffort = (val: string | undefined) => {
    // Selecting `max` clears ultracode (they conflict); other values are kept
    // as-is — `xhigh` + ultracode is the intended pair.
    if (val === "max") patch({ effort: "max", ultracode: undefined });
    else patch({ effort: val });
  };
  const onUltracode = (on: boolean) => {
    // Turning ultracode on only drops a conflicting `max` effort; `xhigh`/etc.
    // survive so toggling ultracode off restores the prior value.
    if (on) patch({ ultracode: true, ...(effortMax ? { effort: undefined } : {}) });
    else patch({ ultracode: undefined });
  };
  // While ultracode is on, effort is pinned to xhigh at runtime — show that and
  // disable the select (the stored value is untouched, so it returns on toggle-off).
  const effortValue = effortDisabled ? "xhigh" : ov.effort ?? "";
  return (
    <div className="flex flex-col gap-2 rounded-lg border border-outline-variant bg-surface-container p-3">
      <div className="flex items-center justify-between">
        <span className="font-code-sm text-code-sm text-on-surface">{title}</span>
        <span className="rounded border border-outline-variant/50 px-1.5 py-0.5 font-label-caps text-label-caps text-on-surface-variant">
          {roleLabel}
        </span>
      </div>
      <label className="flex items-center justify-between gap-3">
        <span className="font-code-sm text-code-sm text-on-surface">
          Effort level
        </span>
        <select
          className={selectClass}
          value={effortValue}
          disabled={effortDisabled}
          onChange={(e) => onEffort(e.target.value || undefined)}
        >
          <option value="">
            Inherit{inheritedEffort ? ` (${inheritedEffort})` : " (default)"}
          </option>
          {EFFORT_OPTS.map((v) => (
            <option key={v} value={v}>
              {v}
            </option>
          ))}
        </select>
      </label>
      {ultracodeOn && (
        <p className="font-label-caps text-label-caps text-on-surface-variant">
          ultracode pins effort to xhigh.
        </p>
      )}
      <ToggleRow
        label="Ultracode"
        checked={ultracodeOn}
        onChange={onUltracode}
        disabled={ultracodeDisabled}
      />
      {effortMax && !isEyes && (
        <p className="font-label-caps text-label-caps text-on-surface-variant">
          max can&apos;t combine with ultracode.
        </p>
      )}
      <p className="font-label-caps text-label-caps text-on-surface-variant">
        {isEyes
          ? "EYES runs without --settings, so ultracode can't be injected. Effort may have limited effect on non-Anthropic models (e.g. DeepSeek)."
          : "ultracode = xhigh + dynamic workflows (Opus 4.8/4.7); higher token use."}
      </p>
    </div>
  );
}

function KnobEditor({
  knob,
  pendingStrings,
  pendingBools,
  stageString,
  stageBool,
}: {
  knob: SettingItem;
  pendingStrings: Record<string, string | null>;
  pendingBools: Record<string, boolean>;
  stageString: (key: string, value: string | null) => void;
  stageBool: (key: string, value: boolean) => void;
}) {
  const editor = KNOB_EDITORS[knob.key];
  if (!editor) {
    return (
      <span className="shrink-0 font-code-sm text-code-sm text-on-surface">
        {knob.value ?? "—"}
      </span>
    );
  }
  if (editor.type === "bool") {
    const checked =
      knob.key in pendingBools ? pendingBools[knob.key] : knob.value === "true";
    return <Switch checked={checked} onChange={(v) => stageBool(knob.key, v)} />;
  }
  // string / enum — effective value = staged value if present, else current.
  const cur = knob.key in pendingStrings ? pendingStrings[knob.key] : knob.value;
  if (editor.type === "enum") {
    const isEffort = knob.key === "env.CLAUDE_CODE_EFFORT_LEVEL";
    return (
      <select
        className={selectClass}
        value={cur ?? ""}
        onChange={(e) => {
          const val = e.target.value || null;
          stageString(knob.key, val);
          // Effort is env-routed; clear the legacy top-level field so it can't
          // shadow the env var (settings.json rejects `max` there anyway).
          if (isEffort) stageString("effortLevel", null);
        }}
      >
        <option value="">unset</option>
        {editor.options!.map((o) => (
          <option key={o} value={o}>
            {o}
          </option>
        ))}
      </select>
    );
  }
  // text (controlled by the staged value)
  return (
    <input
      type="text"
      value={cur ?? ""}
      placeholder="unset"
      onChange={(e) => stageString(knob.key, e.target.value || null)}
      className={cn(selectClass, "w-40")}
    />
  );
}

function SkillsPane({
  config,
  all,
  setSkill,
}: {
  config: ClaudeConfigView;
  all: AgentOverride;
  setSkill: (name: string, vis: SkillVisibility | null) => void;
}) {
  const overrides = all.skills ?? {};
  return (
    <div>
      <PaneHeader
        title="Skills"
        blurb="User skills your agents load. A skill can self-invoke and derail a workflow — set one to 'Manual only' or 'Off' for the agents while keeping it in your own Claude."
      />
      {config.skills.length === 0 ? (
        <Empty>No user skills found under skills/.</Empty>
      ) : (
        <ul className="flex flex-col gap-2">
          {config.skills.map((s: SkillItem) => {
            const ov = overrides[s.name] ?? null;
            return (
              <Row key={s.name} active={ov !== null}>
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="font-code-sm text-code-sm text-on-surface">
                        {s.name}
                      </span>
                      {s.disable_model_invocation && (
                        <span className="rounded border border-outline-variant/50 px-1 font-label-caps text-label-caps text-on-surface-variant">
                          frontmatter: manual-only
                        </span>
                      )}
                    </div>
                    {s.description && (
                      <div className="truncate font-label-caps text-label-caps text-on-surface-variant" title={s.description}>
                        {s.description}
                      </div>
                    )}
                  </div>
                  <select
                    className={selectClass}
                    value={ov ?? ""}
                    onChange={(e) =>
                      setSkill(
                        s.name,
                        (e.target.value || null) as SkillVisibility | null,
                      )
                    }
                  >
                    <option value="">Inherit</option>
                    <option value="on">On</option>
                    <option value="name-only">Name only</option>
                    <option value="user-invocable-only">Manual only</option>
                    <option value="off">Off</option>
                  </select>
                </div>
                <InheritanceBadges inheritance={s.inheritance} />
              </Row>
            );
          })}
        </ul>
      )}
    </div>
  );
}

function PluginsPane({
  config,
  all,
  setPlugin,
  pendingPlugins,
  stagePlugin,
}: {
  config: ClaudeConfigView;
  all: AgentOverride;
  setPlugin: (key: string, enabled: boolean | null) => void;
  pendingPlugins: Record<string, boolean>;
  stagePlugin: (key: string, enabled: boolean) => void;
}) {
  const overrides = all.plugins ?? {};
  return (
    <div>
      <PaneHeader
        title="Plugins & marketplaces"
        blurb="'Your Claude' edits settings.json globally; 'Agents' overrides enablement for new agent sessions only. Disabling a plugin drops its bundled skills/hooks/MCP too."
      />
      {config.plugins.length === 0 ? (
        <Empty>No plugins in enabledPlugins.</Empty>
      ) : (
        <ul className="flex flex-col gap-2">
          {config.plugins.map((p: PluginItem) => {
            const ov = overrides[p.key];
            const hasOv = ov !== undefined;
            return (
              <Row key={p.key} active={hasOv}>
                <div className="flex items-center justify-between gap-3">
                  <div className="min-w-0">
                    <div className="truncate font-code-sm text-code-sm text-on-surface">
                      {p.key}
                    </div>
                    <InheritanceBadges inheritance={p.inheritance} />
                  </div>
                  <div className="flex shrink-0 items-center gap-4">
                    <label className="flex items-center gap-2">
                      <span className="font-label-caps text-label-caps text-on-surface-variant">
                        Your Claude
                      </span>
                      <Switch
                        checked={
                          p.key in pendingPlugins
                            ? pendingPlugins[p.key]
                            : p.enabled
                        }
                        onChange={(v) => stagePlugin(p.key, v)}
                      />
                    </label>
                    <label className="flex items-center gap-2">
                      <span className="font-label-caps text-label-caps text-on-surface-variant">
                        Agents
                      </span>
                      <select
                        className={selectClass}
                        value={hasOv ? (ov ? "on" : "off") : ""}
                        onChange={(e) =>
                          setPlugin(
                            p.key,
                            e.target.value === ""
                              ? null
                              : e.target.value === "on",
                          )
                        }
                      >
                        <option value="">Inherit</option>
                        <option value="on">Force on</option>
                        <option value="off">Off</option>
                      </select>
                    </label>
                  </div>
                </div>
              </Row>
            );
          })}
        </ul>
      )}
    </div>
  );
}

function McpPane({
  config,
  all,
  setMcp,
}: {
  config: ClaudeConfigView;
  all: AgentOverride;
  setMcp: (name: string, forward: boolean | null) => void;
}) {
  const overrides = all.mcp ?? {};
  return (
    <div>
      <PaneHeader
        title="MCP servers"
        blurb="Servers forwarded into Brian (bot-hq + claude-in-chrome are always filtered; Rain gets none). 'settings.json (ignored)' means claude-code itself doesn't load it — but bot-hq still forwards it."
      />
      {config.mcp_servers.length === 0 ? (
        <Empty>No MCP servers configured.</Empty>
      ) : (
        <ul className="flex flex-col gap-2">
          {config.mcp_servers.map((m: McpServerItem) => {
            const ov = overrides[m.name];
            const hasOv = ov !== undefined;
            return (
              <Row key={m.name} active={hasOv}>
                <div className="flex items-center justify-between gap-3">
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="font-code-sm text-code-sm text-on-surface">
                        {m.name}
                      </span>
                      <span className="rounded border border-outline-variant/50 px-1 font-label-caps text-label-caps text-on-surface-variant">
                        {m.transport}
                      </span>
                      {!m.effective && (
                        <span className="rounded border border-amber-500/40 bg-amber-500/10 px-1 font-label-caps text-label-caps text-amber-300">
                          ignored by claude-code
                        </span>
                      )}
                    </div>
                    <div className="truncate font-label-caps text-label-caps text-on-surface-variant" title={m.detail}>
                      {m.detail || m.loaded_from}
                    </div>
                  </div>
                  {m.reserved_filtered ? (
                    <span className="shrink-0 rounded border border-outline-variant/50 px-1.5 py-0.5 font-label-caps text-label-caps text-on-surface-variant">
                      always filtered
                    </span>
                  ) : (
                    <select
                      className={selectClass}
                      value={hasOv ? (ov ? "on" : "off") : ""}
                      onChange={(e) =>
                        setMcp(
                          m.name,
                          e.target.value === ""
                            ? null
                            : e.target.value === "on",
                        )
                      }
                    >
                      <option value="">Inherit (forward)</option>
                      <option value="on">Force forward</option>
                      <option value="off">Off for agents</option>
                    </select>
                  )}
                </div>
                <div className="flex flex-wrap items-center gap-2">
                  <span className="font-label-caps text-label-caps text-on-surface-variant">
                    {m.loaded_from}
                  </span>
                </div>
              </Row>
            );
          })}
        </ul>
      )}
    </div>
  );
}

function MemoryPane({
  config,
  all,
  patchAll,
}: {
  config: ClaudeConfigView;
  all: AgentOverride;
  patchAll: (p: Partial<AgentOverride>) => void;
}) {
  const m = config.memory;
  return (
    <div>
      <PaneHeader
        title="Memory & instructions"
        blurb="CLAUDE.md + auto-memory your agents autodiscover. Suppress them for agents (Brian; Rain already skips them via --bare)."
      />
      <ul className="mb-4 flex flex-col gap-2">
        <FileRow label="User CLAUDE.md" stat={m.user_claude_md} />
        <FileRow label="Home CLAUDE.md (~/CLAUDE.md)" stat={m.home_claude_md} />
        <Row>
          <div className="flex items-center justify-between">
            <span className="font-code-sm text-code-sm text-on-surface">
              Auto-memory projects
            </span>
            <span className="font-code-sm text-code-sm text-on-surface-variant">
              {m.projects_with_memory} with MEMORY.md
            </span>
          </div>
        </Row>
      </ul>
      <InheritanceBadges inheritance={m.inheritance} />
      <div className="mt-4 flex flex-col gap-2 rounded-lg border border-outline-variant bg-surface-container p-3">
        <ToggleRow
          label="Disable auto-memory for agents"
          checked={all.disable_auto_memory === true}
          onChange={(v) =>
            patchAll({ disable_auto_memory: v ? true : undefined })
          }
        />
        <ToggleRow
          label="Disable CLAUDE.md autodiscovery for agents"
          checked={all.disable_claude_md === true}
          onChange={(v) => patchAll({ disable_claude_md: v ? true : undefined })}
        />
      </div>
    </div>
  );
}

function FileRow({
  label,
  stat,
}: {
  label: string;
  stat: { present: boolean; path: string; bytes: number };
}) {
  return (
    <Row>
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <div className="font-code-sm text-code-sm text-on-surface">{label}</div>
          <div className="truncate font-label-caps text-label-caps text-on-surface-variant" title={stat.path}>
            {stat.path}
          </div>
        </div>
        <span
          className={cn(
            "shrink-0 rounded border px-1.5 py-0.5 font-label-caps text-label-caps",
            stat.present
              ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-300"
              : "border-outline-variant/50 text-on-surface-variant",
          )}
        >
          {stat.present ? `${stat.bytes} B` : "absent"}
        </span>
      </div>
    </Row>
  );
}

function PermissionsPane({ config }: { config: ClaudeConfigView }) {
  const p = config.permissions;
  return (
    <div>
      <PaneHeader
        title="Permissions"
        blurb="Your settings.json permission posture (read-only). bot-hq sets each agent's posture independently at spawn, so this does not flow to agents."
      />
      <dl className="grid grid-cols-2 gap-3 sm:grid-cols-3">
        <Stat label="Default mode" value={p.default_mode ?? "default"} />
        <Stat label="Allow rules" value={`${p.allow}`} />
        <Stat label="Ask rules" value={`${p.ask}`} />
        <Stat label="Deny rules" value={`${p.deny}`} />
        <Stat label="Extra dirs" value={`${p.additional_directories}`} />
      </dl>
      <div className="mt-4">
        <InheritanceBadges inheritance={p.inheritance} />
      </div>
    </div>
  );
}

function Switch({
  checked,
  onChange,
  disabled,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => !disabled && onChange(!checked)}
      className={cn(
        "relative inline-flex h-5 w-9 shrink-0 items-center rounded-full border transition-colors",
        disabled ? "cursor-not-allowed opacity-40" : "cursor-pointer",
        checked
          ? "border-primary bg-primary"
          : "border-outline bg-surface-container-highest",
      )}
    >
      {/* Knob color flips with state so it stays visible on both the light
          (on) and dark (off) track. */}
      <span
        className={cn(
          "inline-block h-3.5 w-3.5 transform rounded-full transition-transform",
          checked
            ? "translate-x-[1.125rem] bg-on-primary"
            : "translate-x-0.5 bg-on-surface-variant",
        )}
      />
    </button>
  );
}

function ToggleRow({
  label,
  checked,
  onChange,
  disabled,
}: {
  label: string;
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <label className="flex items-center justify-between gap-3">
      <span className="font-code-sm text-code-sm text-on-surface">{label}</span>
      <Switch checked={checked} onChange={onChange} disabled={disabled} />
    </label>
  );
}

function Empty({ children }: { children: React.ReactNode }) {
  return (
    <div className="rounded-lg border border-outline-variant bg-surface-container p-4 font-code-sm text-code-sm text-on-surface-variant">
      {children}
    </div>
  );
}
