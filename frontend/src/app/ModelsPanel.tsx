import { useState } from "react";
import { keepPreviousData } from "@tanstack/react-query";
import { useTauriQuery, useTauriMutation, errorMessage } from "../hooks/useInvoke";
import { Button } from "../components/ui/Button";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { useFocusTrap } from "../hooks/useFocusTrap";
import { useEscapeKey } from "../hooks/useEscapeKey";
import { cn } from "../lib/cn";
import { formatTimestamp } from "../lib/time";
import { terminalInputClass, FieldLabel } from "./contextLibraryShared";
import { SaveIcon } from "../components/icons";
import type { AccountMark, AccountSetupCommands, ModelView, ValidateResult } from "../lib/bindings";
import { invoke } from "@tauri-apps/api/core";
import { selectClass } from "../components/ui/Select";
import { wideDialogClass } from "../components/ui/Dialog";
import { Skeleton } from "../components/ui/Skeleton";

const PROVIDERS = ["anthropic", "openai", "deepseek", "local"] as const;

// Shared 5-column grid for the header row + each model row. Every track has a
// compressible minmax floor (no fixed-width tracks): the old fixed
// 8rem/9rem/12rem columns gave the row a ~51rem hard minimum, which clipped
// the trailing actions column on narrow windows now that containers hide
// horizontal overflow instead of scrolling.
const rowGridClass =
  "grid grid-cols-[minmax(8rem,1.4fr)_minmax(4.5rem,8rem)_minmax(6rem,1fr)_minmax(5rem,9rem)_minmax(7.5rem,12rem)] items-center gap-3 px-4";

/**
 * Settings → Models. A pure registry of saved LLM endpoints (display name +
 * provider + model id + optional base_url/auth_token). A role names one of
 * these as its default on the Roles tab; the New Session dialog can override it
 * per participant at create time. No default lives here.
 *
 * Rendered as a list; create/edit go through ModelDialog. The row is only
 * upserted when the dialog confirms, so cancelling "Add model" leaves no
 * ghost "New model" entry behind (the old card grid pre-created one).
 */
export function ModelsPanel() {
  const { data: models = [], refetch, isLoading } =
    useTauriQuery<ModelView[]>("list_models");
  const del = useTauriMutation<void, { id: string }>("delete_model");
  // 0091: the usage-limit marks, one per account (config dir + organisation).
  // A row shows the mark for its dir — "limited until …" and a Clear — and
  // nothing else: no pointer at another account (spec §8).
  const { data: marks = [], refetch: refetchMarks } =
    useTauriQuery<AccountMark[]>("list_account_marks");
  const clearMark = useTauriMutation<
    boolean,
    { configDir: string; orgId: string; modelName: string }
  >("clear_account_mark");

  const [dialog, setDialog] = useState<
    { mode: "create" } | { mode: "edit"; model: ModelView } | null
  >(null);
  const [deleteTarget, setDeleteTarget] = useState<ModelView | null>(null);
  // Inline delete error so a rejected delete_model surfaces in the confirm
  // dialog instead of silently failing (the dialog stays open to show it).
  const [deleteError, setDeleteError] = useState<string | null>(null);
  // B5: per-model pre-flight "Test connection" state + last result.
  const [testing, setTesting] = useState<string | null>(null);
  const [testResult, setTestResult] = useState<Record<string, ValidateResult>>({});

  const onTest = async (id: string) => {
    setTesting(id);
    setTestResult((r) => {
      const next = { ...r };
      delete next[id];
      return next;
    });
    try {
      const res = await invoke<ValidateResult>("validate_model", {
        modelId: id,
      });
      setTestResult((r) => ({ ...r, [id]: res }));
    } catch (e) {
      setTestResult((r) => ({
        ...r,
        [id]: { ok: false, message: errorMessage(e) },
      }));
    } finally {
      setTesting(null);
    }
  };

  return (
    <div className="mx-auto h-full max-w-7xl overflow-y-auto overflow-x-hidden px-6 py-6">
      <div className="mb-6 flex items-start justify-between gap-4">
        <div>
          <h2 className="font-headline-lg text-headline-lg text-on-surface">
            Models
          </h2>
          <p className="mt-1 max-w-prose font-body-md text-body-md text-on-surface-variant">
            Saved LLM endpoints. Name one as a role's default on the Roles
            tab, or pick per participant when you create a session.
          </p>
        </div>
        <Button variant="primary" onClick={() => setDialog({ mode: "create" })}>
          + Add model
        </Button>
      </div>

      {isLoading ? (
        <Skeleton
          className="space-y-1"
          rowClassName="h-11 rounded border border-outline-variant bg-surface-container"
        />
      ) : models.length === 0 ? (
        <p className="font-body-md text-body-md text-on-surface-variant">
          No saved models yet. Add one to assign it to an agent.
        </p>
      ) : (
        <div className="overflow-hidden rounded-lg border border-outline-variant bg-surface-container">
          <div
            className={cn(
              rowGridClass,
              "border-b border-outline-variant py-2",
            )}
          >
            <span className="font-label-caps text-label-caps text-on-surface-variant">
              Name
            </span>
            <span className="font-label-caps text-label-caps text-on-surface-variant">
              Provider
            </span>
            <span className="font-label-caps text-label-caps text-on-surface-variant">
              Model id
            </span>
            <span className="font-label-caps text-label-caps text-on-surface-variant">
              Updated
            </span>
            <span aria-hidden />
          </div>
          <div className="divide-y divide-outline-variant/40">
            {models.map((m) => (
              <div key={m.id}>
                <div className={cn(rowGridClass, "py-2.5")}>
                  <span className="truncate font-body-md text-body-md text-on-surface">
                    {m.display_name || "Untitled model"}
                  </span>
                  <span className="truncate font-code-sm text-code-sm text-on-surface-variant">
                    {m.provider || "—"}
                  </span>
                  <span
                    className="flex min-w-0 items-center gap-1.5 font-code-sm text-code-sm text-on-surface-variant"
                    title={m.model_name}
                  >
                    <span className="truncate">{m.model_name || "—"}</span>
                    {m.claude_config_dir && !m.auth_token && !m.base_url && (
                      <span
                        data-testid="model-account"
                        className="shrink-0 rounded border border-outline-variant px-1 font-label-caps text-label-caps text-on-surface-variant"
                        title={`Claude config dir: ${m.claude_config_dir}`}
                      >
                        {accountLabelOf(m)}
                      </span>
                    )}
                  </span>
                  <span className="truncate font-code-sm text-code-sm text-on-surface-variant">
                    {m.updated_at ? formatTimestamp(m.updated_at) : "—"}
                  </span>
                  <div className="flex flex-wrap justify-end gap-2">
                    <Button
                      size="sm"
                      disabled={testing === m.id}
                      title="Pre-flight check this model's token + gateway"
                      onClick={() => onTest(m.id)}
                    >
                      {testing === m.id ? "…" : "Test"}
                    </Button>
                    <Button
                      size="sm"
                      onClick={() => setDialog({ mode: "edit", model: m })}
                    >
                      Edit
                    </Button>
                    <Button
                      variant="danger"
                      size="sm"
                      disabled={del.isPending}
                      onClick={() => {
                        setDeleteError(null);
                        setDeleteTarget(m);
                      }}
                    >
                      Delete
                    </Button>
                  </div>
                </div>
                {marksFor(m, marks).map((mark) => (
                  <div
                    key={`${mark.config_dir}|${mark.org_id}|${mark.model_name}`}
                    data-testid="model-limit"
                    className="flex flex-wrap items-center gap-2 px-4 pb-2 font-code-sm text-code-sm text-on-surface-variant"
                  >
                    <span className="break-words">
                      {limitLine(mark)}
                    </span>
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={clearMark.isPending}
                      title="Forget this limit — e.g. you enabled extra usage, or the reset passed"
                      onClick={async () => {
                        await clearMark.mutateAsync({
                          configDir: mark.config_dir,
                          orgId: mark.org_id,
                          modelName: mark.model_name,
                        });
                        await refetchMarks();
                      }}
                    >
                      Clear
                    </Button>
                  </div>
                ))}
                {testResult[m.id] && (
                  <div
                    className={cn(
                      "px-4 pb-2 font-code-sm text-code-sm",
                      testResult[m.id].ok ? "text-success" : "text-error",
                    )}
                  >
                    {testResult[m.id].ok ? "✓ " : "✗ "}
                    {testResult[m.id].message}
                  </div>
                )}
              </div>
            ))}
          </div>
        </div>
      )}

      {dialog && (
        <ModelDialog
          initial={dialog.mode === "edit" ? dialog.model : null}
          onClose={() => setDialog(null)}
          onSaved={() => {
            setDialog(null);
            refetch();
          }}
        />
      )}
      <ConfirmDialog
        open={deleteTarget !== null}
        title="Delete saved model?"
        message={
          <>
            Delete{" "}
            <strong className="text-on-surface">
              {deleteTarget?.display_name || "this model"}
            </strong>
            ? This also removes its stored auth token and can&apos;t be undone.
            {deleteError && (
              <span className="mt-3 block rounded border border-error/40 bg-error-container/20 px-3 py-2 text-on-error-container">
                Delete failed: {deleteError}
              </span>
            )}
          </>
        }
        confirmLabel="Delete"
        confirmVariant="danger"
        onConfirm={async () => {
          if (!deleteTarget) return;
          setDeleteError(null);
          try {
            await del.mutateAsync({ id: deleteTarget.id });
            setDeleteTarget(null);
            refetch();
          } catch (e) {
            // Keep the dialog open so the inline error is visible.
            setDeleteError(errorMessage(e));
          }
        }}
        onCancel={() => {
          setDeleteError(null);
          setDeleteTarget(null);
        }}
      />
    </div>
  );
}

function emptyDraft(): ModelView {
  return {
    id: "",
    display_name: "",
    provider: "anthropic",
    model_name: "",
    base_url: null,
    auth_token: null,
    created_at: "",
    updated_at: "",
    context_window: null,
    cli_settings: null,
    claude_config_dir: null,
  };
}

/** The dir key a subscription row's marks are stored under (`""` = default). */
function dirKeyOf(m: Pick<ModelView, "claude_config_dir">): string {
  return (m.claude_config_dir ?? "").trim().replace(/[\\/]+$/, "");
}

/** The limit marks that apply to a row: a subscription row's dir AND model
 *  (claude.ai limits are per model — a Fable limit says nothing about the
 *  same account's Opus row), whatever organisation was signed in there. A
 *  gateway row has no account. */
export function marksFor(
  m: Pick<ModelView, "claude_config_dir" | "auth_token" | "base_url" | "model_name">,
  marks: AccountMark[],
): AccountMark[] {
  if ((m.auth_token && m.auth_token.length > 0) || (m.base_url && m.base_url.length > 0)) {
    return [];
  }
  const key = dirKeyOf(m);
  return marks.filter((mark) => mark.config_dir === key && mark.model_name === m.model_name);
}

/** "limited until <local time> — <the CLI's line>", and only that. */
export function limitLine(mark: AccountMark): string {
  const who = mark.email ? `${mark.email}: ` : "";
  const until = mark.limited_until
    ? `limited until about ${new Date(mark.limited_until).toLocaleString()}`
    : "limited (no reset time given)";
  return `${who}${until} — "${mark.limited_text}"`;
}

/** How the dialog and the list name the account a row bills: blank dir =
 *  the default `~/.claude`; a gateway row (its own token or base URL) bills
 *  its provider, not a Claude account. */
export function accountLabelOf(m: Pick<ModelView, "claude_config_dir" | "auth_token" | "base_url" | "provider">): string {
  if ((m.auth_token && m.auth_token.length > 0) || (m.base_url && m.base_url.length > 0)) {
    return m.provider;
  }
  const dir = (m.claude_config_dir ?? "").trim();
  if (!dir) return "default";
  const parts = dir.replace(/[\\/]+$/, "").split(/[\\/]/);
  return parts[parts.length - 1] || dir;
}

// ============================================================================
// ModelDialog — create (initial=null) or edit one saved model. The id is
// generated at save time for creates, so cancelling never persists anything.
// ============================================================================

function ModelDialog({
  initial,
  onClose,
  onSaved,
}: {
  initial: ModelView | null;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [draft, setDraft] = useState<ModelView>(initial ?? emptyDraft());
  const [tokenVisible, setTokenVisible] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const upsert = useTauriMutation<void, { model: ModelView }>("upsert_model");
  const trapRef = useFocusTrap<HTMLDivElement>();
  // Escape closes, mirroring ConfirmDialog (conditionally mounted — no guard).
  useEscapeKey(onClose);

  const title = initial ? "Edit model" : "Add model";
  const providerIsCustom = !PROVIDERS.includes(
    draft.provider as (typeof PROVIDERS)[number],
  );
  const canSave = !upsert.isPending && draft.display_name.trim().length > 0;

  // The one-time setup for a second account's dir, written by the backend in
  // this machine's shell (sh, or PowerShell on Windows) and checked by Save's
  // rules first — a `~` path comes back as the refusal, not a command. The
  // block shows on `wantsSetup` alone: with the previous dir's commands kept
  // while the next resolve (no flashing per keystroke), `data` can outlive a
  // cleared field or a token typed since.
  const setupDir = (draft.claude_config_dir ?? "").trim();
  const wantsSetup = setupDir.length > 0 && !draft.auth_token && !draft.base_url;
  const setup = useTauriQuery<AccountSetupCommands>(
    "account_setup_commands",
    { dir: setupDir },
    { enabled: wantsSetup, placeholderData: keepPreviousData, retry: false },
  );
  const powershell = setup.data?.shell === "powershell";

  const submit = async () => {
    if (!canSave) return;
    setError(null);
    try {
      await upsert.mutateAsync({
        model: { ...draft, id: draft.id || crypto.randomUUID() },
      });
      onSaved();
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  return (
    <>
      {/* Scrim — the frame's SIBLING, not its parent (see `wideDialogClass`):
          a drag-select across the setup commands that is released out here
          must not count as a click that closes the dialog and drops the
          draft. */}
      <div
        className="fixed inset-0 z-40 bg-black/60"
        onClick={onClose}
        aria-hidden
      />
      <div
        ref={trapRef}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        className={wideDialogClass}
      >
        <div className="mb-4 flex shrink-0 items-center justify-between">
          <h2 className="font-headline-md text-headline-md text-on-surface">
            {title}
          </h2>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="text-on-surface-variant hover:text-on-surface"
          >
            ×
          </button>
        </div>

        {/* Two columns, as New session: the model — its id and context
            window — and the endpoint it is reached at on the left; the
            account it bills and the claude CLI settings on the right. Each
            column scrolls itself on md+; below md the whole body scrolls as
            one stack. The error line and the actions sit outside the body,
            so a refusal is never below the fold. */}
        <div
          data-testid="model-dialog-body"
          className="min-h-0 flex-1 gap-x-6 overflow-x-hidden max-md:space-y-4 max-md:overflow-y-auto md:flex"
        >
          <div className="min-h-0 space-y-4 overflow-x-hidden md:w-[340px] md:shrink-0 md:overflow-y-auto md:pr-1">
            <label className="block">
              <FieldLabel>Display name</FieldLabel>
              <input
                type="text"
                value={draft.display_name}
                onChange={(e) =>
                  setDraft({ ...draft, display_name: e.target.value })
                }
                placeholder="e.g. Opus (Anthropic)"
                autoFocus
                className={terminalInputClass}
              />
            </label>

            <label className="block">
              <FieldLabel>Provider</FieldLabel>
              <select
                value={providerIsCustom ? "other" : draft.provider}
                onChange={(e) =>
                  setDraft({
                    ...draft,
                    provider: e.target.value === "other" ? "" : e.target.value,
                  })
                }
                className={selectClass}
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
              <FieldLabel>Model id</FieldLabel>
              <input
                type="text"
                value={draft.model_name}
                onChange={(e) =>
                  setDraft({ ...draft, model_name: e.target.value })
                }
                placeholder="claude-opus-5"
                className={terminalInputClass}
              />
            </label>

            <label className="block">
              <FieldLabel>Context window</FieldLabel>
              <input
                type="number"
                min={1}
                value={draft.context_window ?? ""}
                onChange={(e) =>
                  setDraft({
                    ...draft,
                    context_window: e.target.value
                      ? Number(e.target.value)
                      : null,
                  })
                }
                placeholder="(unknown — meter shows a gap)"
                className={terminalInputClass}
              />
              <span className="mt-1 block break-words font-body text-code-sm text-on-surface-variant">
                Total tokens this specific model accepts. The context meter still
                takes its window from claude-code, which reports one per turn; this
                value is what that report is checked <strong>against</strong> — when
                the two disagree, the session gets a notice naming both numbers.
              </span>
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
              <FieldLabel>Auth token</FieldLabel>
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

            {/* The "Native loop" checkbox lived here until rc3 D9. bot-hq now has
                one connector, so there is no runtime to choose — but the choice it
                used to make still has a consequence the user has to be able to
                see, which is what this says. Test lives on the saved row:
                `validate_model` checks a stored row, never this draft. */}
            <p className="break-words rounded border border-outline-variant/60 bg-surface-container-lowest p-2 font-body text-code-sm text-on-surface-variant">
              Every saved model is spawned through the <strong>claude CLI</strong>,
              so its endpoint has to speak the Anthropic Messages API. A gateway
              that does not will fail at spawn — save it, then press{" "}
              <strong>Test</strong> on its row to find out now instead of
              mid-session.
            </p>
          </div>

          <div className="min-h-0 min-w-0 space-y-4 overflow-x-hidden md:flex-1 md:overflow-y-auto md:pr-1">
            <label className="block">
              <FieldLabel>Claude config dir (second account)</FieldLabel>
              <input
                type="text"
                value={draft.claude_config_dir ?? ""}
                onChange={(e) =>
                  setDraft({ ...draft, claude_config_dir: e.target.value || null })
                }
                placeholder="(blank = the default ~/.claude)"
                spellCheck={false}
                className={terminalInputClass}
              />
              <span className="mt-1 block break-words font-body text-code-sm text-on-surface-variant">
                Which Claude subscription a participant on this model bills. Leave
                it blank for the account signed in to <code>~/.claude</code>. To run
                a second subscription beside it, give it its own absolute path (one
                signed-in account per dir); a participant is spawned into that dir
                and stays there for its whole life. Ignored for a gateway model —
                its token bills the gateway.
              </span>
            </label>

            {wantsSetup && setup.isError && (
              <p
                data-testid="account-setup-refusal"
                className="break-words font-code-sm text-code-sm text-error"
              >
                {errorMessage(setup.error)}
              </p>
            )}
            {wantsSetup && !setup.isError && setup.data && (
              <div className="rounded border border-outline-variant/60 bg-surface-container-lowest p-2">
                <p className="mb-1 break-words font-body text-code-sm text-on-surface-variant">
                  One-time setup, in {powershell ? "PowerShell" : "your own terminal"} —
                  sign the second account in to this dir (bot-hq never runs it), then
                  press <strong>Test</strong> on the saved row to confirm:
                </p>
                <pre
                  data-testid="account-setup-command"
                  className="select-all overflow-x-hidden whitespace-pre-wrap break-all rounded bg-surface-container p-2 font-code-sm text-code-sm text-on-surface"
                >
                  {setup.data.setup}
                </pre>
                <p className="mb-1 mt-2 break-words font-body text-code-sm text-on-surface-variant">
                  Optional: share the default dir&apos;s CLAUDE.md, settings
                  (plugins, model overrides), skills and commands with it. Session
                  history and memory stay per account, and a re-run skips what is
                  already linked.
                  {powershell && (
                    <>
                      {" "}The folders link as junctions; CLAUDE.md and settings.json
                      need Developer Mode (search “For developers” in Settings) or an
                      administrator PowerShell.
                    </>
                  )}
                </p>
                <pre
                  data-testid="account-share-command"
                  className="select-all overflow-x-hidden whitespace-pre-wrap break-all rounded bg-surface-container p-2 font-code-sm text-code-sm text-on-surface"
                >
                  {setup.data.share}
                </pre>
              </div>
            )}

            <label className="block">
              <FieldLabel>Claude CLI settings (JSON)</FieldLabel>
              <textarea
                value={draft.cli_settings ?? ""}
                onChange={(e) =>
                  setDraft({ ...draft, cli_settings: e.target.value || null })
                }
                placeholder='{"modelOverrides":{"claude-fable-5":"claude-fable-5-1"}}'
                rows={3}
                spellCheck={false}
                className={cn(terminalInputClass, "resize-y whitespace-pre-wrap break-all")}
              />
              <span className="mt-1 block break-words font-body text-code-sm text-on-surface-variant">
                Merged into every participant&apos;s <code>--settings</code> at
                spawn, executor and reviewer alike. Use it when the installed claude
                CLI does not know this model id and runs it at its 200k default:
                map a model id the CLI does know to this one under{" "}
                <code>modelOverrides</code>. Must be a JSON object; a role&apos;s own
                Claude-config override wins on any key both set.
              </span>
            </label>
          </div>
        </div>

        {error && (
          <p className="mt-3 shrink-0 break-words font-code-sm text-code-sm text-error">
            {error}
          </p>
        )}

        <div className="mt-5 flex shrink-0 items-center justify-end gap-2">
          <Button type="button" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            type="button"
            variant="primary"
            disabled={!canSave}
            onClick={submit}
          >
            <SaveIcon />
            {upsert.isPending ? "Saving…" : "Save"}
          </Button>
        </div>
      </div>
    </>
  );
}
