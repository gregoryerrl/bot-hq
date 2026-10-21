import { Fragment, useState } from "react";
import { keepPreviousData } from "@tanstack/react-query";
import { useTauriQuery, useTauriMutation, errorMessage } from "../hooks/useInvoke";
import { Button } from "../components/ui/Button";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { cn } from "../lib/cn";
import { formatTimestamp } from "../lib/time";
import { accountLabelOf, formatLimitMark, markIsLive, marksFor } from "../lib/accounts";
import { FieldLabel } from "./contextLibraryShared";
import { CheckIcon, CrossIcon, SaveIcon } from "../components/icons";
import type { AccountMark, AccountSetupCommands, ModelView, ValidateResult } from "../lib/bindings";
import { invoke } from "@tauri-apps/api/core";
import { Dialog } from "../components/ui/Dialog";
import { Skeleton } from "../components/ui/Skeleton";
import { fieldClass } from "../components/ui/field";
import { SettingsPage } from "../components/ui/SettingsPage";

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
  // A failed Clear used to reject silently (a bare `await mutateAsync`). The
  // error belongs under the row whose Clear failed, keyed by the model row and
  // the mark — one list-wide string showed it under every marked row.
  const [clearError, setClearError] = useState<{ key: string; message: string } | null>(null);
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
    <SettingsPage>
      <div className="mb-6 flex items-start justify-between gap-4">
        <div>
          <h2 className="font-headline-lg text-headline-lg text-on-surface">
            Models
          </h2>
          <p className="mt-1 max-w-prose font-body-md text-body-md text-muted">
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
        <p className="font-body-md text-body-md text-muted">
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
              <div key={m.id} data-testid="model-row">
                <div className={cn(rowGridClass, "py-2.5")}>
                  <span className="truncate font-body-md text-body-md text-on-surface">
                    {m.display_name || "Untitled model"}
                  </span>
                  <span className="flex min-w-0 items-center gap-1.5 font-code-sm text-code-sm text-on-surface-variant">
                    <span className="truncate">{m.provider || "—"}</span>
                    {m.runtime === "codex" && (
                      <span
                        data-testid="model-runtime"
                        className="shrink-0 rounded border border-outline-variant px-1 font-label-caps text-label-caps text-on-surface-variant"
                        title="Spawned through the Codex CLI (codex app-server)"
                      >
                        codex
                      </span>
                    )}
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
                        title={`${accountDirLabel(m.runtime)}: ${m.claude_config_dir}`}
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
                {marksFor(m, marks).map((mark) => {
                  const markKey = `${mark.config_dir}|${mark.org_id}|${mark.model_name}`;
                  const errorKey = `${m.id}|${markKey}`;
                  return (
                  <Fragment key={markKey}>
                  <div
                    data-testid="model-limit"
                    className="flex flex-wrap items-center gap-2 px-4 pb-2 font-code-sm text-code-sm"
                  >
                    <span className="break-words">
                      <span className={markIsLive(mark) ? "text-warning" : "text-muted"}>
                        {mark.email ? `${mark.email}: ` : ""}
                        {formatLimitMark(mark).headline}
                      </span>
                      <span className="text-muted"> — "{mark.limited_text}"</span>
                    </span>
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={clearMark.isPending}
                      title="Forget this limit — e.g. you enabled extra usage, or the reset passed"
                      onClick={async () => {
                        setClearError(null);
                        try {
                          await clearMark.mutateAsync({
                            configDir: mark.config_dir,
                            orgId: mark.org_id,
                            modelName: mark.model_name,
                          });
                          await refetchMarks();
                        } catch (e) {
                          setClearError({ key: errorKey, message: errorMessage(e) });
                        }
                      }}
                    >
                      Clear
                    </Button>
                  </div>
                  {clearError?.key === errorKey && (
                    <p role="alert" className="px-4 pb-2 font-code-sm text-code-sm text-error">
                      Couldn't clear the limit: {clearError.message}
                    </p>
                  )}
                  </Fragment>
                  );
                })}
                {testResult[m.id] && (
                  <div
                    className={cn(
                      "px-4 pb-2 font-code-sm text-code-sm",
                      testResult[m.id].ok ? "text-success" : "text-error",
                    )}
                  >
                    {testResult[m.id].ok ? (
                      <CheckIcon className="mr-1 inline-block size-3 align-middle" />
                    ) : (
                      <CrossIcon className="mr-1 inline-block size-3 align-middle" />
                    )}
                    <span className="sr-only">{testResult[m.id].ok ? "Passed: " : "Failed: "}</span>
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
    </SettingsPage>
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
    runtime: "claude_code",
  };
}

/** What the account-dir field is called per connector: the column is one
 *  (`claude_config_dir`), the slot it names is the CLI's — `CLAUDE_CONFIG_DIR`
 *  for claude-code, `CODEX_HOME` for Codex. */
function accountDirLabel(runtime: string): string {
  return runtime === "codex" ? "Codex home" : "Claude config dir";
}

// The account and limit-mark rules live in `lib/accounts` (shared with the
// New Session dialog); these names stay for the tests and callers here.
export { accountLabelOf, marksFor };

/** "<email>: limited until <local time> — <the CLI's line>", and only that. */
export function limitLine(mark: AccountMark): string {
  const { headline, detail } = formatLimitMark(mark);
  const who = mark.email ? `${mark.email}: ` : "";
  return `${who}${headline} — ${detail.slice(who.length)}`;
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
  const codex = draft.runtime === "codex";
  const setup = useTauriQuery<AccountSetupCommands>(
    "account_setup_commands",
    { dir: setupDir, runtime: draft.runtime },
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

  // The `lg` Dialog — the New session frame (see `ui/Dialog`'s `lg`). Its scrim
  // is the frame's SIBLING, not its parent: a drag-select across the setup
  // commands that is released out there must not count as a click that closes
  // the dialog and drops the draft. Its body is a flex column so the
  // two-column block can take the body's height.
  return (
    <Dialog
      open
      onClose={onClose}
      title={title}
      size="lg"
      bodyClassName="flex flex-col"
      footer={
        <>
          {error && (
            <p className="w-full min-w-0 break-words font-code-sm text-code-sm text-error">
              {error}
            </p>
          )}
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
        </>
      }
    >
      {/* Two columns, as New session: the model — its id and context
          window — and the endpoint it is reached at on the left; the
          account it bills and the claude CLI settings on the right. Each
          column scrolls itself on md+; below md the whole body scrolls as
          one stack. The error line and the actions sit in the footer,
          outside the body, so a refusal is never below the fold. */}
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
              className={fieldClass}
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
              className={fieldClass}
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
                className={cn("mt-2", fieldClass)}
              />
            )}
          </label>

          <label className="block">
            <FieldLabel>Runtime</FieldLabel>
            <select
              value={draft.runtime}
              // A dir belongs to one runtime (a Claude dir must never become
              // a Codex home, nor the reverse — the backend refuses it), so a
              // runtime change starts the account field over.
              onChange={(e) =>
                setDraft({
                  ...draft,
                  runtime: e.target.value,
                  claude_config_dir: e.target.value === draft.runtime ? draft.claude_config_dir : null,
                })
              }
              className={fieldClass}
            >
              <option value="claude_code">Claude Code (claude CLI)</option>
              <option value="codex">Codex (OpenAI Codex CLI)</option>
            </select>
            <span className="mt-1 block break-words font-body-md text-body-md text-on-surface-variant">
              Which agent CLI a participant on this model runs through. A session
              mixes them freely — pick this row for one role and a Claude row for
              another.
            </span>
          </label>

          <label className="block">
            <FieldLabel>Model id</FieldLabel>
            <input
              type="text"
              value={draft.model_name}
              onChange={(e) =>
                setDraft({ ...draft, model_name: e.target.value })
              }
              placeholder={codex ? "gpt-6-astra" : "claude-opus-5"}
              className={fieldClass}
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
              className={fieldClass}
            />
            <span className="mt-1 block break-words font-body-md text-body-md text-on-surface-variant">
              The provider&apos;s published limit for this model, in tokens. For a
              model id that is not Claude&apos;s own (OpenRouter, DeepSeek, …) this
              IS the window the claude CLI runs it at — bot-hq hands it over as{" "}
              <code>CLAUDE_CODE_MAX_CONTEXT_TOKENS</code>; leave it blank and the
              CLI uses its own default. For a Claude id the CLI keeps its own
              catalog and this value is only checked against what it reports:
              when the two disagree, the session gets a notice naming both
              numbers.
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
              className={fieldClass}
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
                className={cn(fieldClass, "pr-12")}
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
          {codex ? (
            <p className="break-words rounded border border-outline-variant/60 bg-surface-container-lowest p-2 font-body-md text-body-md text-muted">
              This model is spawned through the <strong>Codex CLI</strong>{" "}
              (<code>codex app-server</code>) and bills the ChatGPT account signed
              in to its Codex home. The base URL and auth token are not used;
              the model id is one the account can run (<code>gpt-6-astra</code>,{" "}
              <code>gpt-6.1-sol</code>). Save it, then press <strong>Test</strong>{" "}
              on its row to confirm the sign-in and the model.
            </p>
          ) : (
            <p className="break-words rounded border border-outline-variant/60 bg-surface-container-lowest p-2 font-body-md text-body-md text-muted">
              Every saved model is spawned through the <strong>claude CLI</strong>,
              so its endpoint has to speak the Anthropic Messages API. A gateway
              that does not will fail at spawn — save it, then press{" "}
              <strong>Test</strong> on its row to find out now instead of
              mid-session.
            </p>
          )}
        </div>

        <div className="min-h-0 min-w-0 space-y-4 overflow-x-hidden md:flex-1 md:overflow-y-auto md:pr-1">
          <label className="block">
            <FieldLabel>{codex ? "Codex home" : "Claude config dir (second account)"}</FieldLabel>
            <input
              type="text"
              value={draft.claude_config_dir ?? ""}
              onChange={(e) =>
                setDraft({ ...draft, claude_config_dir: e.target.value || null })
              }
              placeholder={codex ? "/Users/<you>/.codex-bot-hq" : "(blank = the default ~/.claude)"}
              spellCheck={false}
              className={fieldClass}
            />
            {codex ? (
              <span className="mt-1 block break-words font-body-md text-body-md text-on-surface-variant">
                The <code>CODEX_HOME</code> a participant on this model is spawned
                with — the ChatGPT account signed in there is the one it bills.
                Give bot-hq a home of its own (an absolute path, not{" "}
                <code>~/.codex</code>): bot-hq writes the hooks that enforce the
                Tool Gate into it, and its threads live there. One signed-in
                account per home; a participant stays in its home for its whole
                life.
              </span>
            ) : (
              <span className="mt-1 block break-words font-body-md text-body-md text-on-surface-variant">
                Which Claude subscription a participant on this model bills. Leave
                it blank for the account signed in to <code>~/.claude</code>. To run
                a second subscription beside it, give it its own absolute path (one
                signed-in account per dir); a participant is spawned into that dir
                and stays there for its whole life. Ignored for a gateway model —
                its token bills the gateway.
              </span>
            )}
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
              <p className="mb-1 break-words font-body-md text-body-md text-muted">
                One-time setup, in {powershell ? "PowerShell" : "your own terminal"} —
                sign the {codex ? "ChatGPT" : "second"} account in to this{" "}
                {codex ? "home" : "dir"} (bot-hq never runs it), then press{" "}
                <strong>Test</strong> on the saved row to confirm:
              </p>
              <pre
                data-testid="account-setup-command"
                className="select-all overflow-x-hidden whitespace-pre-wrap break-all rounded bg-surface-container p-2 font-code-sm text-code-sm text-on-surface"
              >
                {setup.data.setup}
              </pre>
              {setup.data.share && (
              <>
              <p className="mb-1 mt-2 break-words font-body-md text-body-md text-muted">
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
              </>
              )}
            </div>
          )}

          {!codex && (
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
              className={cn(fieldClass, "resize-y whitespace-pre-wrap break-all")}
            />
            <span className="mt-1 block break-words font-body-md text-body-md text-on-surface-variant">
              Merged into every participant&apos;s <code>--settings</code> at
              spawn, executor and reviewer alike. Use it for a <em>Claude</em> id
              the installed claude CLI does not know yet: under{" "}
              <code>modelOverrides</code>, map a Claude id the CLI does know
              (ideally the same family) to this one, and the CLI applies that
              model&apos;s window. Not for a gateway model — the CLI would send the
              mapped Claude model&apos;s request profile, which the gateway rejects
              (OpenRouter: <code>400 configuration_update is not supported</code>);
              a non-Claude model gets its window from the row&apos;s own limit
              field instead (the number on the left). Must be a JSON object; a
              role&apos;s own Claude-config override wins on any key both set.
            </span>
          </label>
          )}
        </div>
      </div>
    </Dialog>
  );
}
