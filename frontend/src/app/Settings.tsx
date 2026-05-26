import { useState } from "react";
import { useTauriQuery, useTauriMutation } from "../hooks/useInvoke";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import { Card, CardTitle } from "../components/ui/Card";
import { authorColorClass } from "../components/AuthorBadge";
import type { AgentConfigView } from "../lib/bindings";

export function Settings() {
  const { data: configs = [], refetch } = useTauriQuery<AgentConfigView[]>(
    "list_agent_configs",
  );
  const upsert = useTauriMutation<void, { cfg: AgentConfigView }>(
    "upsert_agent_config",
  );

  return (
    <div className="mx-auto max-w-3xl px-6 py-6">
      <h1 className="mb-4 text-xl font-semibold">Agent configuration</h1>
      <p className="mb-6 max-w-prose text-sm text-neutral-400">
        Per-agent provider, model, and auth token. Plaintext token storage in
        v1 — keychain migration tracked separately.
      </p>
      <div className="space-y-4">
        {configs.map((c) => (
          <AgentRow
            key={c.agent_name}
            cfg={c}
            onSave={async (next) => {
              await upsert.mutateAsync({ cfg: next });
              refetch();
            }}
          />
        ))}
      </div>
    </div>
  );
}

function AgentRow({
  cfg,
  onSave,
}: {
  cfg: AgentConfigView;
  onSave: (next: AgentConfigView) => Promise<void>;
}) {
  const [draft, setDraft] = useState(cfg);
  const dirty = JSON.stringify(draft) !== JSON.stringify(cfg);

  return (
    <Card>
      <div className="mb-3 flex items-center gap-2">
        <span
          className={`size-2 rounded-full bg-current ${authorColorClass(cfg.agent_name)}`}
        />
        <CardTitle>{cfg.agent_name}</CardTitle>
      </div>
      <div className="grid gap-3 md:grid-cols-2">
        <label className="block">
          <span className="mb-1 block text-xs text-neutral-400">Provider</span>
          <Input
            value={draft.provider}
            onChange={(e) => setDraft({ ...draft, provider: e.target.value })}
          />
        </label>
        <label className="block">
          <span className="mb-1 block text-xs text-neutral-400">Model</span>
          <Input
            value={draft.model_name}
            onChange={(e) =>
              setDraft({ ...draft, model_name: e.target.value })
            }
          />
        </label>
        <label className="block">
          <span className="mb-1 block text-xs text-neutral-400">Base URL</span>
          <Input
            value={draft.base_url ?? ""}
            onChange={(e) =>
              setDraft({ ...draft, base_url: e.target.value || null })
            }
            placeholder="(default)"
          />
        </label>
        <label className="block">
          <span className="mb-1 flex items-center justify-between text-xs text-neutral-400">
            <span>Auth token</span>
            <span className="text-amber-400">⚠ plaintext</span>
          </span>
          <Input
            type="password"
            value={draft.auth_token ?? ""}
            onChange={(e) =>
              setDraft({ ...draft, auth_token: e.target.value || null })
            }
            placeholder="(unset)"
          />
        </label>
      </div>
      <div className="mt-3 flex justify-end gap-2">
        <Button variant="ghost" disabled={!dirty} onClick={() => setDraft(cfg)}>
          Reset
        </Button>
        <Button
          variant="primary"
          disabled={!dirty}
          onClick={() => onSave(draft)}
        >
          Save
        </Button>
      </div>
    </Card>
  );
}
