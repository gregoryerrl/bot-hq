import { useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { PRIVACY_URL, type TelemetryStatus } from "../lib/telemetry";
import { useTauriQuery, useTauriMutation } from "../hooks/useInvoke";
import { useServerDraft } from "../hooks/useServerDraft";
import { Button } from "../components/ui/Button";
import { SubTabButton } from "../components/SubTabButton";
import { cn } from "../lib/cn";
import { formatTimestamp } from "../lib/time";
import { terminalInputClass } from "./contextLibraryShared";
import { SaveIcon, WarnIcon } from "../components/icons";
import { ClaudeConfigPanel } from "./ClaudeConfig";
import { ModelsPanel } from "./ModelsPanel";
import { RolesPanel } from "./RolesPanel";
import { ViolationsPanel } from "./ViolationsPanel";
import { FeedbackPanel } from "./FeedbackPanel";
import { PromptcodesPanel } from "./PromptcodesPanel";
import { PresetOfferCard } from "./PresetOfferCard";
import { PolicyForm } from "../components/PolicyForm";
import { GatedKeywordList } from "../components/GatedKeywordList";
import {
  osNotificationsEnabled,
  setOsNotificationsEnabled,
} from "../lib/osNotifications";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import type {
  GatedKeyword,
  Policy,
  SessionInfo,
  UpdateInfo,
} from "../lib/bindings";
import { shortSessionId } from "../lib/sessionId";
import { Skeleton } from "../components/ui/Skeleton";

type SettingsSubTab =
  | "roles"
  | "models"
  | "claude"
  | "toolgate"
  | "policy"
  | "violations"
  | "feedback"
  | "promptcodes"
  | "archive"
  | "updates"
  | "notifications"
  | "diagnostics";

/**
 * Settings is a tabbed container. Every panel that has been visited stays
 * mounted (toggled with `hidden`) so in-progress edits survive a subtab switch.
 *
 * rc3 D8 retired the "Agents" subtab: a role owns its default model
 * (`roles.default_model_id`, the Roles tab) and the New Session dialog
 * overrides it per participant, so there is nothing left for a per-agent
 * model card to configure. Roles is the landing tab because it is the surface
 * that replaced it.
 */
const SUBTABS: readonly SettingsSubTab[] = [
  "roles",
  "models",
  "claude",
  "toolgate",
  "policy",
  "violations",
  "feedback",
  "promptcodes",
  "archive",
  "updates",
];

export function Settings() {
  // `?tab=toolgate` deep-links a subtab (the Dashboard's preset-offer banner
  // uses it — landing a first-run user on Roles when the offer card is on
  // Tool Gate would be a dead end). Unknown values fall back to the landing
  // tab.
  const [params] = useSearchParams();
  const requested = params.get("tab") as SettingsSubTab | null;
  const initial: SettingsSubTab =
    requested && SUBTABS.includes(requested) ? requested : "roles";
  const [tab, setTab] = useState<SettingsSubTab>(initial);
  // Lazy-mount-then-keep: a panel mounts only once its tab has been visited, then
  // STAYS mounted (CSS `hidden` when inactive) so in-progress edits survive a
  // subtab switch. This skips firing the queries of tabs the user never opens —
  // the old code mounted all 6 panels (and all their queries) on first visit.
  const [visited, setVisited] = useState<Set<SettingsSubTab>>(
    () => new Set<SettingsSubTab>([initial]),
  );
  const select = (t: SettingsSubTab) => {
    setTab(t);
    setVisited((v) => (v.has(t) ? v : new Set(v).add(t)));
  };
  return (
    <div className="flex h-full flex-col bg-background">
      <div
        role="tablist"
        aria-label="Settings tabs"
        className="flex shrink-0 items-center gap-1 border-b border-outline-variant px-4"
      >
        <SubTabButton
          active={tab === "roles"}
          controls="settings-tab-roles"
          onClick={() => select("roles")}
        >
          Roles
        </SubTabButton>
        <SubTabButton
          active={tab === "models"}
          controls="settings-tab-models"
          onClick={() => select("models")}
        >
          Models
        </SubTabButton>
        <SubTabButton
          active={tab === "claude"}
          controls="settings-tab-claude"
          onClick={() => select("claude")}
        >
          Claude Config
        </SubTabButton>
        <SubTabButton
          active={tab === "toolgate"}
          controls="settings-tab-toolgate"
          onClick={() => select("toolgate")}
        >
          Tool Gate
        </SubTabButton>
        <SubTabButton
          active={tab === "policy"}
          controls="settings-tab-policy"
          onClick={() => select("policy")}
        >
          Policy
        </SubTabButton>
        <SubTabButton
          active={tab === "violations"}
          controls="settings-tab-violations"
          onClick={() => select("violations")}
        >
          Violations
        </SubTabButton>
        <SubTabButton
          active={tab === "feedback"}
          controls="settings-tab-feedback"
          onClick={() => select("feedback")}
        >
          Feedback
        </SubTabButton>
        <SubTabButton
          active={tab === "promptcodes"}
          controls="settings-tab-promptcodes"
          onClick={() => select("promptcodes")}
        >
          Promptcodes
        </SubTabButton>
        <SubTabButton
          active={tab === "archive"}
          controls="settings-tab-archive"
          onClick={() => select("archive")}
        >
          Archive
        </SubTabButton>
        <SubTabButton
          active={tab === "updates"}
          controls="settings-tab-updates"
          onClick={() => select("updates")}
        >
          Updates
        </SubTabButton>
        <SubTabButton
          active={tab === "notifications"}
          controls="settings-tab-notifications"
          onClick={() => select("notifications")}
        >
          Notifications
        </SubTabButton>
        <SubTabButton
          active={tab === "diagnostics"}
          controls="settings-tab-diagnostics"
          onClick={() => select("diagnostics")}
        >
          Diagnostics
        </SubTabButton>
      </div>
      <div className="min-h-0 flex-1">
        <div
          id="settings-tab-roles"
          role="tabpanel"
          className={cn("h-full", tab !== "roles" && "hidden")}
        >
          {visited.has("roles") && <RolesPanel />}
        </div>
        <div
          id="settings-tab-models"
          role="tabpanel"
          className={cn("h-full", tab !== "models" && "hidden")}
        >
          {visited.has("models") && <ModelsPanel />}
        </div>
        <div
          id="settings-tab-claude"
          role="tabpanel"
          className={cn("h-full", tab !== "claude" && "hidden")}
        >
          {visited.has("claude") && <ClaudeConfigPanel />}
        </div>
        <div
          id="settings-tab-toolgate"
          role="tabpanel"
          className={cn("h-full", tab !== "toolgate" && "hidden")}
        >
          {visited.has("toolgate") && <ToolGatePanel />}
        </div>
        <div
          id="settings-tab-policy"
          role="tabpanel"
          className={cn("h-full", tab !== "policy" && "hidden")}
        >
          {visited.has("policy") && <GlobalPolicyPanel />}
        </div>
        <div
          id="settings-tab-violations"
          role="tabpanel"
          className={cn("h-full", tab !== "violations" && "hidden")}
        >
          {visited.has("violations") && <ViolationsPanel />}
        </div>
        <div
          id="settings-tab-feedback"
          role="tabpanel"
          className={cn("h-full", tab !== "feedback" && "hidden")}
        >
          {visited.has("feedback") && <FeedbackPanel />}
        </div>
        <div
          id="settings-tab-promptcodes"
          role="tabpanel"
          className={cn("h-full", tab !== "promptcodes" && "hidden")}
        >
          {visited.has("promptcodes") && <PromptcodesPanel />}
        </div>
        <div
          id="settings-tab-archive"
          role="tabpanel"
          className={cn("h-full", tab !== "archive" && "hidden")}
        >
          {visited.has("archive") && <ArchivePanel />}
        </div>
        <div
          id="settings-tab-updates"
          role="tabpanel"
          className={cn("h-full", tab !== "updates" && "hidden")}
        >
          {visited.has("updates") && <UpdatesPanel />}
        </div>
        <div
          id="settings-tab-notifications"
          role="tabpanel"
          className={cn("h-full", tab !== "notifications" && "hidden")}
        >
          {visited.has("notifications") && <NotificationsPanel />}
        </div>
        <div
          id="settings-tab-diagnostics"
          role="tabpanel"
          className={cn("h-full", tab !== "diagnostics" && "hidden")}
        >
          {visited.has("diagnostics") && <DiagnosticsPanel />}
        </div>
      </div>
    </div>
  );
}

function ToolGatePanel() {
  return (
    <div className="mx-auto h-full max-w-7xl overflow-y-auto overflow-x-hidden px-6 py-6">
      <PresetOfferCard kind="gates" />
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
    try {
      await save.mutateAsync({ policy: draft });
    } catch {
      return; // `save.error` renders below; no unhandled rejection
    }
    refetch();
  };

  return (
    <div className="mx-auto h-full max-w-4xl overflow-y-auto overflow-x-hidden px-6 py-6">
      <PresetOfferCard kind="policy" />
      <div className="mb-6 flex items-start justify-between gap-4">
        <div>
          <h2 className="font-headline-lg text-headline-lg text-on-surface">
            Global Policy
          </h2>
          <p className="mt-1 max-w-prose font-body-md text-body-md text-on-surface-variant">
            The base policy every project and session inherits at spawn
            (<code>general-policy.yaml</code>). Projects can tighten it in the
            Context Library; a live session can override it in the gear panel.
          </p>
        </div>
        {dirty && (
          <Button
            type="button"
            variant="primary"
            onClick={onSave}
            disabled={save.isPending}
            className="shrink-0"
          >
            <SaveIcon />
            {save.isPending ? "Saving…" : "Save policy"}
          </Button>
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
      <SessionDefaults />
    </div>
  );
}

/**
 * App-wide defaults applied at session create.
 *
 * These are `app_settings` rows, not policy YAML, and each toggle persists on
 * change — the panel's "Save policy" button does not cover them. They lived
 * under the Agents subtab until rc3 D8 retired it; the worktree default is not
 * per-agent configuration, so it moved here rather than being deleted with it.
 *
 * rc3 D13 removed the second toggle ("start new sessions with one
 * participant"). The user: *"There's no 'disable rain by default' on rc3, thats
 * moot. Just don't add the role to your session creation."* It was a toggle
 * only because the roster was fixed at two; now the New-session dialog picks
 * the roster, so starting solo is just not adding a second participant.
 *
 * The `rain_disabled_default` key and its backend readers **are gone** — every
 * remaining mention in `src/` is a past-tense record of the deletion, and
 * `Settings.test.tsx:126` pins that this panel never asks for the key at all.
 * This line said "a parallel unit deletes" it until round 4: a merge-era
 * instruction that outlived its merge, the same shape as the one that left
 * `ParticipantView` three fields short.
 */
function SessionDefaults() {
  const { data: worktreeDefault, refetch } = useTauriQuery<string | null>(
    "get_app_setting",
    { key: "worktree_default" },
  );
  // `adherence_nudges` (storage/models.rs): the opt-out for the Track-A
  // nudges — the pre-Apply mutation reminder, the Plan→Apply nudge and the
  // close-out learnings ask (the CL-opener nudge at first spawn was dead since
  // D29 and removed in round 12). Read in three places and, until round 8,
  // settable nowhere but SQLite.
  const { data: adherenceNudges, refetch: refetchNudges } = useTauriQuery<
    string | null
  >("get_app_setting", { key: "adherence_nudges" });
  const setAppSetting = useTauriMutation<void, { key: string; value: string }>(
    "set_app_setting",
  );
  return (
    <section className="mt-gutter rounded-lg border border-outline-variant bg-surface-container p-4">
      <h3 className="font-headline-md text-headline-md text-on-surface">
        Session defaults
      </h3>
      <label className="mt-3 flex items-center gap-2">
        <input
          type="checkbox"
          checked={worktreeDefault !== "0"}
          onChange={async (e) => {
            try {
              await setAppSetting.mutateAsync({
                key: "worktree_default",
                value: e.target.checked ? "1" : "0",
              });
              refetch();
            } catch {
              // rendered below
            }
          }}
          className="size-4 accent-primary"
        />
        <span className="font-body-md text-body-md text-on-surface">
          Run repo-backed sessions in isolated git worktrees
        </span>
      </label>
      <p className="mt-1 font-code-sm text-code-sm text-on-surface-variant">
        Each session gets its own checkout on branch{" "}
        <code className="text-on-surface">bothq/&lt;session-id&gt;</code>, so
        several sessions can work the same project in parallel. Clean worktrees
        are removed at close; anything uncommitted is kept. The New-session
        dialog can override this per session.
      </p>
      <label className="mt-3 flex items-center gap-2">
        <input
          type="checkbox"
          checked={adherenceNudges !== "0"}
          onChange={async (e) => {
            try {
              await setAppSetting.mutateAsync({
                key: "adherence_nudges",
                value: e.target.checked ? "1" : "0",
              });
              refetchNudges();
            } catch {
              // rendered below
            }
          }}
          className="size-4 accent-primary"
        />
        <span className="font-body-md text-body-md text-on-surface">
          Send adherence nudges to participants
        </span>
      </label>
      <p className="mt-1 font-code-sm text-code-sm text-on-surface-variant">
        The one-time reminders bot-hq posts into a session: open the Context
        Library first, mutations belong in Apply, the Plan→Apply hand-off, and
        the close-out learnings ask. Off = none of them; the tools and gates
        are unaffected.
      </p>
      {setAppSetting.error && (
        <p className="mt-2 inline-block rounded border border-error/40 bg-error-container/20 px-2 py-1 font-code-sm text-code-sm text-on-error-container">
          Couldn’t save: {setAppSetting.error.message}
        </p>
      )}
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
      <WarnIcon size={12} className="mr-1 inline-block align-[-2px]" />
      Worktree kept
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
        <h2 className="font-headline-lg text-headline-lg text-on-surface">
          Archived Sessions
        </h2>
        <p className="mt-1 max-w-prose font-body-md text-body-md text-on-surface-variant">
          Every closed session. Click one to review its history — read-only,
          nothing respawns. <strong>Reopen</strong> (in the session view)
          brings its participants back via <code>--resume</code> with their
          prior context when claude-code still has it, and puts it back on
          the dashboard.
        </p>
      </div>
      {isLoading ? (
        <Skeleton
          className="space-y-2"
          rowClassName="h-14 rounded-lg border border-outline-variant bg-surface-container"
        />
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
                      {shortSessionId(s.id)}
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
          <h2 className="font-headline-lg text-headline-lg text-on-surface">
            Updates
          </h2>
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
    try {
      await save.mutateAsync({
        keywords: draft.filter((k) => k.keyword.trim() !== ""),
      });
    } catch {
      return; // `save.error` renders below; no unhandled rejection
    }
    refetch();
  };

  return (
    <section className="mt-10 border-t border-outline-variant/30 pt-6">
      <div className="mb-4 flex items-start justify-between gap-4">
        <div>
          <h3 className="font-headline-md text-headline-md text-on-surface">
            Gated Bash Keywords
          </h3>
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
          <Button
            type="button"
            variant="primary"
            onClick={onSave}
            disabled={save.isPending}
            className="shrink-0"
          >
            <SaveIcon />
            {save.isPending ? "Saving…" : "Save keywords"}
          </Button>
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
// Notifications — escalate needs-you moments to the OS while unfocused
// ============================================================================

function NotificationsPanel() {
  const [enabled, setEnabled] = useState(osNotificationsEnabled());
  const [testState, setTestState] = useState<"idle" | "sent" | "denied" | "failed">(
    "idle",
  );
  // Some(false) only when Windows' master switch is explicitly off; null on
  // every other platform (the signal doesn't exist there — render nothing).
  const { data: toastEnabled = null } = useTauriQuery<boolean | null>(
    "windows_toast_enabled",
  );
  const toastMasterOff = toastEnabled === false;

  const toggle = () => {
    const next = !enabled;
    setOsNotificationsEnabled(next);
    setEnabled(next);
    if (next) {
      // Flipping On is the focused, intentional moment — request the OS
      // permission HERE. The lazy request at first fire only ever runs while
      // the user is in another app, where a surprise system prompt gets
      // dismissed — and a dismissal is sticky.
      void (async () => {
        try {
          let granted = await isPermissionGranted();
          if (!granted) granted = (await requestPermission()) === "granted";
          setTestState(granted ? "idle" : "denied");
        } catch {
          setTestState("failed");
        }
      })();
    }
  };

  const sendTest = async () => {
    try {
      let granted = await isPermissionGranted();
      if (!granted) granted = (await requestPermission()) === "granted";
      if (!granted) {
        setTestState("denied");
        return;
      }
      sendNotification({
        title: "bot-hq — test notification",
        body: "This is what a needs-you escalation looks like.",
      });
      setTestState("sent");
    } catch {
      setTestState("failed");
    }
  };

  return (
    <div className="mx-auto h-full max-w-4xl overflow-y-auto overflow-x-hidden px-6 py-6">
      <h2 className="font-headline-lg text-headline-lg text-on-surface">
        Notifications
      </h2>
      <p className="mt-1 max-w-prose font-body-md text-body-md text-on-surface-variant">
        While the window is unfocused, bot-hq escalates needs-you moments to
        your machine&rsquo;s notifications. The in-app bell counts parked
        questions; OS escalation deliberately covers more — questions and
        approvals, gated commands, and session halts. Repeats are
        cooldown-suppressed and simultaneous waits coalesce into one summary.
      </p>

      <div className="mt-6 flex items-center justify-between gap-4 rounded-lg border border-outline-variant bg-surface-container p-5">
        <div>
          <div className="font-body-md text-body-md text-on-surface">
            OS notifications
          </div>
          <div className="mt-0.5 max-w-prose font-code-sm text-code-sm text-on-surface-variant">
            {enabled
              ? "On — escalations toast while the window is unfocused."
              : "Off — everything stays in the in-app bell, tray and banners."}
          </div>
        </div>
        <button
          type="button"
          role="switch"
          aria-checked={enabled}
          onClick={toggle}
          className={cn(
            "inline-flex shrink-0 items-center rounded border px-3 py-1.5 font-code-sm text-code-sm transition-colors",
            enabled
              ? "border-primary bg-primary text-on-primary hover:bg-primary-fixed"
              : "border-outline-variant text-on-surface-variant hover:bg-surface-container-high hover:text-on-surface",
          )}
        >
          {enabled ? "On" : "Off"}
        </button>
      </div>

      <div className="mt-4 flex items-center gap-3">
        <button
          type="button"
          onClick={() => void sendTest()}
          className="inline-flex items-center rounded border border-outline-variant px-3 py-1.5 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:bg-surface-container-high hover:text-on-surface"
        >
          Send test notification
        </button>
        {testState === "sent" && (
          <span className="font-code-sm text-code-sm text-on-surface-variant">
            Sent — if nothing appeared, check this app&rsquo;s permission in
            your OS notification settings.
          </span>
        )}
        {testState === "denied" && (
          <span className="font-code-sm text-code-sm text-warning">
            Permission denied — grant it in your OS notification settings.
          </span>
        )}
        {testState === "failed" && (
          <span className="font-code-sm text-code-sm text-warning">
            Send failed — on Linux this usually means no notification daemon is
            running.
          </span>
        )}
      </div>
      {/* Windows-only: the ToastEnabled registry value is the ONE signal
          that carries information about delivery — the plugin's permission
          API is a desktop compile-time constant and the send is
          fire-and-forget, so without this a disabled OS master switch reads
          as a broken app (it did, for a full release). */}
      {toastMasterOff && (
        <p
          role="alert"
          className="mt-3 max-w-prose rounded border border-warning/40 bg-warning-container/20 px-3 py-2 font-code-sm text-code-sm text-on-surface"
        >
          Windows notifications are OFF at the OS level — no app can display a
          toast. Enable them under System Settings → Notifications, then send
          the test again.
        </p>
      )}
    </div>
  );
}

// ============================================================================
// Diagnostics — opt-in telemetry (status, toggle, endpoint, privacy)
// ============================================================================

function DiagnosticsPanel() {
  const status = useTauriQuery<TelemetryStatus>("get_telemetry_status", {});
  const [endpointDraft, setEndpointDraft] = useState<string | null>(null);
  const [endpointError, setEndpointError] = useState<string | null>(null);

  const s = status.data;
  const call = async (cmd: string, args?: Record<string, unknown>) => {
    try {
      setEndpointError(null);
      await invoke(cmd, args);
    } catch (e) {
      setEndpointError(String(e));
    } finally {
      void status.refetch();
    }
  };

  return (
    <div className="mx-auto h-full max-w-4xl overflow-y-auto overflow-x-hidden px-6 py-6">
      <h2 className="font-headline-lg text-headline-lg text-on-surface">
        Diagnostics
      </h2>
      <p className="mt-1 max-w-prose font-body-md text-body-md text-on-surface-variant">
        Strictly opt-in. When enabled, bot-hq sends anonymous crash reports
        (hashes, never text), app version + OS, and error classes to an
        endpoint operated by the bot-hq author on Cloudflare — never code,
        prompts, or session content.{" "}
        <button
          type="button"
          onClick={() => void openUrl(PRIVACY_URL)}
          className="underline decoration-outline-variant underline-offset-2 hover:text-primary"
        >
          Full privacy note
        </button>
      </p>

      <div className="mt-6 flex items-center justify-between gap-4 rounded-lg border border-outline-variant bg-surface-container p-5">
        <div>
          <div className="font-body-md text-body-md text-on-surface">
            Share diagnostics
          </div>
          <div className="mt-0.5 max-w-prose font-code-sm text-code-sm text-on-surface-variant">
            {s?.enabled
              ? "On — thank you. Disable any time; the install id and queue die with it."
              : "Off — nothing is collected or sent."}
          </div>
        </div>
        <button
          type="button"
          role="switch"
          aria-checked={s?.enabled ?? false}
          disabled={!s}
          onClick={() => void call("set_telemetry_enabled", { enabled: !s?.enabled })}
          className={cn(
            "inline-flex shrink-0 items-center rounded border px-3 py-1.5 font-code-sm text-code-sm transition-colors",
            s?.enabled
              ? "border-primary bg-primary text-on-primary hover:bg-primary-fixed"
              : "border-outline-variant text-on-surface-variant hover:bg-surface-container-high hover:text-on-surface",
          )}
        >
          {s?.enabled ? "On" : "Off"}
        </button>
      </div>

      <div className="mt-4 rounded-lg border border-outline-variant bg-surface-container p-5">
        <dl className="flex flex-col gap-2 font-code-sm text-code-sm">
          <div className="flex justify-between gap-4">
            <dt className="text-on-surface-variant">Install id</dt>
            <dd className="text-on-surface">
              {s?.install_id ?? "— (minted when you enable)"}
            </dd>
          </div>
          <div className="flex justify-between gap-4">
            <dt className="text-on-surface-variant">Queued locally</dt>
            <dd className="text-on-surface">{s ? `${s.queued_bytes} bytes` : "—"}</dd>
          </div>
        </dl>

        <label className="mt-4 block border-t border-outline-variant/30 pt-4">
          <span className="font-label-caps text-label-caps text-on-surface-variant">
            Endpoint (self-host override)
          </span>
          <div className="mt-1 flex gap-2">
            <input
              value={endpointDraft ?? s?.endpoint ?? ""}
              onChange={(e) => setEndpointDraft(e.target.value)}
              placeholder="default: the author-operated sink (see privacy note)"
              spellCheck={false}
              className="min-w-0 flex-1 rounded border border-outline-variant bg-surface-container-lowest px-3 py-1.5 font-code-sm text-code-sm text-on-surface caret-primary placeholder:text-on-surface-variant focus:border-primary focus:outline-none"
            />
            <button
              type="button"
              disabled={endpointDraft === null}
              onClick={() => {
                void call("set_telemetry_endpoint", { endpoint: endpointDraft ?? "" });
                setEndpointDraft(null);
              }}
              className="inline-flex shrink-0 items-center rounded border border-outline-variant px-3 py-1.5 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:bg-surface-container-high hover:text-on-surface disabled:opacity-50"
            >
              Save
            </button>
          </div>
          <span className="mt-1 block max-w-prose font-code-sm text-code-sm text-on-surface-variant">
            Deploy your own sink from{" "}
            <span className="text-on-surface">packaging/telemetry-worker/</span>{" "}
            and paste its URL to keep diagnostics entirely yours. Empty = the
            default.
          </span>
          {endpointError && (
            <span className="mt-1 block font-code-sm text-code-sm text-warning">
              {endpointError}
            </span>
          )}
        </label>
      </div>
    </div>
  );
}
