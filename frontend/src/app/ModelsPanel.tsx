import { useState } from "react";
import { useTauriQuery, useTauriMutation, errorMessage } from "../hooks/useInvoke";
import { Button } from "../components/ui/Button";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { useFocusTrap } from "../hooks/useFocusTrap";
import { cn } from "../lib/cn";
import { formatTimestamp } from "../lib/time";
import { terminalInputClass, SaveIcon } from "./contextLibraryShared";
import type { ModelView } from "../lib/bindings";

const selectClass =
  "w-full rounded border border-outline-variant bg-surface-container-lowest px-2 py-1.5 font-code-sm text-code-sm text-on-surface focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary";

const PROVIDERS = ["anthropic", "openai", "deepseek", "local"] as const;

// Shared 5-column grid for the header row + each model row.
const rowGridClass =
  "grid grid-cols-[minmax(10rem,1.4fr)_8rem_minmax(8rem,1fr)_9rem_8.5rem] items-center gap-3 px-4";

function FieldLabel({ children }: { children: React.ReactNode }) {
  return (
    <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
      {children}
    </span>
  );
}

/**
 * Settings → Models. A pure registry of saved LLM endpoints (display name +
 * provider + model id + optional base_url/auth_token). Agents pick from these
 * on their own card in the Agents tab; sessions pick at create time. No default
 * lives here — each agent's selected model on the Agents tab IS its default.
 *
 * Rendered as a list; create/edit go through ModelDialog. The row is only
 * upserted when the dialog confirms, so cancelling "Add model" leaves no
 * ghost "New model" entry behind (the old card grid pre-created one).
 */
export function ModelsPanel() {
  const { data: models = [], refetch, isLoading } =
    useTauriQuery<ModelView[]>("list_models");
  const del = useTauriMutation<void, { id: string }>("delete_model");

  const [dialog, setDialog] = useState<
    { mode: "create" } | { mode: "edit"; model: ModelView } | null
  >(null);
  const [deleteTarget, setDeleteTarget] = useState<ModelView | null>(null);

  return (
    <div className="mx-auto h-full max-w-7xl overflow-auto px-6 py-6">
      <div className="mb-6 flex items-start justify-between gap-4">
        <div>
          <h1 className="font-headline-lg text-headline-lg text-on-surface">
            Models
          </h1>
          <p className="mt-1 max-w-prose font-body-md text-body-md text-on-surface-variant">
            Saved LLM endpoints. Assign one to each agent on the Agents tab, or
            pick per-agent when you create a session.
          </p>
        </div>
        <Button variant="primary" onClick={() => setDialog({ mode: "create" })}>
          + Add model
        </Button>
      </div>

      {isLoading ? (
        <div className="space-y-1">
          {[0, 1, 2].map((i) => (
            <div
              key={i}
              className="h-11 animate-pulse rounded border border-outline-variant bg-surface-container"
            />
          ))}
        </div>
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
              <div key={m.id} className={cn(rowGridClass, "py-2.5")}>
                <span className="truncate font-body-md text-body-md text-on-surface">
                  {m.display_name || "Untitled model"}
                </span>
                <span className="truncate font-code-sm text-code-sm text-on-surface-variant">
                  {m.provider || "—"}
                </span>
                <span
                  className="truncate font-code-sm text-code-sm text-on-surface-variant"
                  title={m.model_name}
                >
                  {m.model_name || "—"}
                </span>
                <span className="truncate font-code-sm text-code-sm text-on-surface-variant">
                  {m.updated_at ? formatTimestamp(m.updated_at) : "—"}
                </span>
                <div className="flex justify-end gap-2">
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
                    onClick={() => setDeleteTarget(m)}
                  >
                    Delete
                  </Button>
                </div>
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
          </>
        }
        confirmLabel="Delete"
        confirmVariant="danger"
        onConfirm={async () => {
          if (!deleteTarget) return;
          await del.mutateAsync({ id: deleteTarget.id });
          setDeleteTarget(null);
          refetch();
        }}
        onCancel={() => setDeleteTarget(null)}
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
  };
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

  const title = initial ? "Edit model" : "Add model";
  const providerIsCustom = !PROVIDERS.includes(
    draft.provider as (typeof PROVIDERS)[number],
  );
  const canSave = !upsert.isPending && draft.display_name.trim().length > 0;

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
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4"
      role="dialog"
      aria-modal="true"
      aria-label={title}
      onClick={onClose}
    >
      <div
        ref={trapRef}
        tabIndex={-1}
        className="w-full max-w-md rounded border border-outline-variant bg-surface-container p-4 shadow-lg focus:outline-none"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="mb-4 font-headline-md text-headline-md text-on-surface">
          {title}
        </h2>

        <div className="flex flex-col gap-4">
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
              placeholder="claude-opus-4-8"
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
        </div>

        {error && (
          <p className="mt-3 font-code-sm text-code-sm text-error">{error}</p>
        )}

        <div className="mt-5 flex items-center justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="rounded border border-outline-variant bg-transparent px-3 py-1 font-code-sm text-code-sm text-on-surface-variant transition-colors hover:bg-surface-container-high hover:text-on-surface"
          >
            Cancel
          </button>
          <button
            type="button"
            disabled={!canSave}
            onClick={submit}
            className="inline-flex items-center gap-1.5 rounded border border-primary bg-primary px-3 py-1 font-code-sm text-code-sm text-on-primary transition-colors hover:bg-primary-fixed disabled:opacity-50"
          >
            <SaveIcon />
            {upsert.isPending ? "Saving…" : "Save"}
          </button>
        </div>
      </div>
    </div>
  );
}
