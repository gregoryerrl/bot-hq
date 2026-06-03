import { useState } from "react";
import { useTauriQuery, useTauriMutation } from "../hooks/useInvoke";
import { Button } from "../components/ui/Button";
import { cn } from "../lib/cn";
import { formatTimestamp } from "../lib/time";
import { terminalInputClass, SaveIcon } from "./contextLibraryShared";
import type { ModelView } from "../lib/bindings";

const selectClass =
  "w-full rounded border border-outline-variant bg-surface-container-lowest px-2 py-1.5 font-code-sm text-code-sm text-on-surface focus:border-primary focus:outline-none focus:ring-1 focus:ring-primary";

const PROVIDERS = ["anthropic", "openai", "deepseek", "local"] as const;

function FieldLabel({ children }: { children: React.ReactNode }) {
  return (
    <span className="mb-1 block font-label-caps text-label-caps text-on-surface-variant">
      {children}
    </span>
  );
}

/**
 * Settings → Models. A user-managed registry of LLM endpoints. Each saved model
 * bundles provider + model id + optional base_url/auth_token; sessions pick a
 * model for Brian and Rain at create time (defaulting to the one chosen here).
 * Emma is out of scope — she keeps her own per-agent config on the Agents tab.
 */
export function ModelsPanel() {
  const { data: models = [], refetch, isLoading } =
    useTauriQuery<ModelView[]>("list_models");
  const { data: defaultId, refetch: refetchDefault } = useTauriQuery<
    string | null
  >("get_default_model_id");

  const { data: rainDisabledDefault, refetch: refetchRainDefault } =
    useTauriQuery<string | null>("get_app_setting", {
      key: "rain_disabled_default",
    });

  const upsert = useTauriMutation<void, { model: ModelView }>("upsert_model");
  const setDefault = useTauriMutation<void, { id: string }>(
    "set_default_model_id",
  );
  const setAppSetting = useTauriMutation<void, { key: string; value: string }>(
    "set_app_setting",
  );

  const addModel = async () => {
    const id = crypto.randomUUID();
    await upsert.mutateAsync({
      model: {
        id,
        display_name: "New model",
        provider: "anthropic",
        model_name: "",
        base_url: null,
        auth_token: null,
        created_at: "",
        updated_at: "",
      },
    });
    refetch();
  };

  return (
    <div className="mx-auto h-full max-w-7xl overflow-auto px-6 py-6">
      <div className="mb-6 flex items-start justify-between gap-4">
        <div>
          <h1 className="font-headline-lg text-headline-lg text-on-surface">
            Models
          </h1>
          <p className="mt-1 max-w-prose font-body-md text-body-md text-on-surface-variant">
            Saved LLM endpoints. Sessions pick a model for Brian and Rain at
            create time; the default below is pre-selected for new sessions.
          </p>
        </div>
        <Button variant="primary" onClick={addModel} disabled={upsert.isPending}>
          + Add model
        </Button>
      </div>

      {/* Default model for new sessions */}
      <div className="mb-6 flex flex-wrap items-center gap-3 rounded-lg border border-outline-variant bg-surface-container px-4 py-3">
        <span className="font-label-caps text-label-caps text-on-surface-variant">
          Default for new sessions
        </span>
        <select
          value={defaultId ?? ""}
          onChange={async (e) => {
            await setDefault.mutateAsync({ id: e.target.value });
            refetchDefault();
          }}
          disabled={models.length === 0}
          className={cn(selectClass, "max-w-xs")}
        >
          <option value="">(none)</option>
          {models.map((m) => (
            <option key={m.id} value={m.id}>
              {m.display_name}
            </option>
          ))}
        </select>
        <label className="ml-auto flex items-center gap-2">
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
            Disable Rain by default
          </span>
        </label>
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
      ) : models.length === 0 ? (
        <p className="font-body-md text-body-md text-on-surface-variant">
          No saved models yet. Add one to assign it to Brian or Rain on a new
          session.
        </p>
      ) : (
        <div className="grid grid-cols-1 gap-gutter xl:grid-cols-3">
          {models.map((m) => (
            <ModelCard
              key={m.id}
              model={m}
              isDefault={m.id === defaultId}
              onSaved={refetch}
              onDeleted={() => {
                refetch();
                refetchDefault();
              }}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function ModelCard({
  model,
  isDefault,
  onSaved,
  onDeleted,
}: {
  model: ModelView;
  isDefault: boolean;
  onSaved: () => void;
  onDeleted: () => void;
}) {
  const [draft, setDraft] = useState(model);
  const [tokenVisible, setTokenVisible] = useState(false);
  const upsert = useTauriMutation<void, { model: ModelView }>("upsert_model");
  const del = useTauriMutation<void, { id: string }>("delete_model");

  const dirty = JSON.stringify(draft) !== JSON.stringify(model);
  const providerIsCustom = !PROVIDERS.includes(
    draft.provider as (typeof PROVIDERS)[number],
  );

  return (
    <section className="flex flex-col rounded-lg border border-outline-variant bg-surface-container p-4">
      <div className="mb-4 flex items-center justify-between gap-2">
        <h2 className="truncate font-headline-md text-headline-md text-on-surface">
          {draft.display_name || "Untitled model"}
        </h2>
        {isDefault && (
          <span className="shrink-0 rounded border border-primary/40 bg-primary/15 px-1.5 py-0.5 font-label-caps text-label-caps text-primary">
            Default
          </span>
        )}
      </div>

      <div className="flex flex-1 flex-col gap-4">
        <label className="block">
          <FieldLabel>Display name</FieldLabel>
          <input
            type="text"
            value={draft.display_name}
            onChange={(e) =>
              setDraft({ ...draft, display_name: e.target.value })
            }
            placeholder="e.g. Opus (Anthropic)"
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
              onChange={(e) => setDraft({ ...draft, provider: e.target.value })}
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
            onChange={(e) => setDraft({ ...draft, model_name: e.target.value })}
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

      <div className="mt-4 flex items-center justify-between gap-2 border-t border-outline-variant/30 pt-3">
        <span className="truncate font-code-sm text-code-sm text-on-surface-variant">
          {model.updated_at ? `updated ${formatTimestamp(model.updated_at)}` : ""}
        </span>
        <div className="flex shrink-0 gap-2">
          <Button
            variant="danger"
            size="sm"
            disabled={del.isPending}
            onClick={async () => {
              await del.mutateAsync({ id: model.id });
              onDeleted();
            }}
          >
            Delete
          </Button>
          <button
            type="button"
            disabled={!dirty || upsert.isPending}
            onClick={async () => {
              await upsert.mutateAsync({ model: draft });
              onSaved();
            }}
            className="inline-flex items-center gap-1.5 rounded border border-primary bg-primary px-3 py-1.5 font-code-sm text-code-sm text-on-primary transition-colors hover:bg-primary-fixed disabled:opacity-50"
          >
            <SaveIcon />
            {upsert.isPending ? "Saving…" : "Save"}
          </button>
        </div>
      </div>
    </section>
  );
}
