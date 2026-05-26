import { useCallback, useEffect, useRef, useState } from "react";
import { useBlocker } from "react-router-dom";
import { useTauriQuery, useTauriMutation } from "../hooks/useInvoke";
import { Button } from "../components/ui/Button";
import { Input } from "../components/ui/Input";
import { Card, CardTitle } from "../components/ui/Card";
import { authorColorClass } from "../components/AuthorBadge";
import { cn } from "../lib/cn";
import type { AgentConfigView } from "../lib/bindings";

export function Settings() {
  const { data: configs = [], refetch, isLoading } = useTauriQuery<
    AgentConfigView[]
  >("list_agent_configs");
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
    <div className="mx-auto h-full max-w-3xl overflow-auto px-6 py-6">
      {blocker.state === "blocked" && (
        <div className="mb-4 flex items-center gap-3 rounded-lg border border-amber-500/40 bg-amber-500/5 px-4 py-3">
          <p className="flex-1 text-sm text-amber-200">
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
          <h1 className="text-xl font-semibold tracking-tight">
            Agent configuration
          </h1>
          <p className="mt-1 max-w-prose text-sm text-neutral-400">
            Per-agent provider, model, base URL, and auth token. Tokens are
            stored as plaintext in sqlite for v1 — OS keychain migration is
            tracked separately. Brian + Rain spawn with these settings on next
            session start.
          </p>
        </div>
        {dirtyCount > 0 && (
          <Button
            variant="primary"
            size="sm"
            onClick={() => setSaveAllSignal((n) => n + 1)}
            disabled={upsert.isPending}
          >
            Save all ({dirtyCount})
          </Button>
        )}
      </div>
      {isLoading ? (
        <div className="space-y-3">
          {[0, 1, 2].map((i) => (
            <div
              key={i}
              className="h-40 animate-pulse rounded-lg border border-default bg-surface"
            />
          ))}
        </div>
      ) : (
        <div className="space-y-4">
          {configs.map((c) => (
            <AgentRow
              key={c.agent_name}
              cfg={c}
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

function AgentRow({
  cfg,
  onSave,
  onDirtyChange,
  isSaving,
  saveAllSignal,
}: {
  cfg: AgentConfigView;
  onSave: (next: AgentConfigView) => Promise<void>;
  onDirtyChange: (dirty: boolean) => void;
  isSaving?: boolean;
  saveAllSignal: number;
}) {
  const [draft, setDraft] = useState(cfg);
  const [tokenVisible, setTokenVisible] = useState(false);
  const [saved, setSaved] = useState(false);
  const dirty = JSON.stringify(draft) !== JSON.stringify(cfg);
  const accentDotClass = authorColorClass(cfg.agent_name);

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

  return (
    <Card className="bg-surface">
      <div className="mb-4 flex items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <span className={cn("size-2 rounded-full bg-current", accentDotClass)} />
          <CardTitle className="capitalize">{cfg.agent_name}</CardTitle>
          {dirty && (
            <span className="rounded-full bg-amber-500/15 px-1.5 py-0.5 text-[0.6rem] font-semibold uppercase tracking-wide text-amber-300">
              Unsaved
            </span>
          )}
          {saved && !dirty && (
            <span className="rounded-full bg-emerald-500/15 px-1.5 py-0.5 text-[0.6rem] font-semibold uppercase tracking-wide text-emerald-300">
              Saved ✓
            </span>
          )}
        </div>
        <span className="text-[0.65rem] text-neutral-500">
          updated {cfg.updated_at}
        </span>
      </div>
      <div className="grid gap-3 md:grid-cols-2">
        <label className="block">
          <span className="mb-1 block text-xs text-neutral-400">Provider</span>
          <Input
            value={draft.provider}
            onChange={(e) => setDraft({ ...draft, provider: e.target.value })}
            placeholder="anthropic"
          />
        </label>
        <label className="block">
          <span className="mb-1 block text-xs text-neutral-400">Model</span>
          <Input
            value={draft.model_name}
            onChange={(e) =>
              setDraft({ ...draft, model_name: e.target.value })
            }
            placeholder="claude-opus-4-7"
          />
        </label>
        <label className="block">
          <span className="mb-1 block text-xs text-neutral-400">Base URL</span>
          <Input
            value={draft.base_url ?? ""}
            onChange={(e) =>
              setDraft({ ...draft, base_url: e.target.value || null })
            }
            placeholder="(provider default)"
          />
        </label>
        <label className="block">
          <span className="mb-1 block text-xs text-neutral-400">Auth token</span>
          <div className="relative">
            <Input
              type={tokenVisible ? "text" : "password"}
              value={draft.auth_token ?? ""}
              onChange={(e) =>
                setDraft({ ...draft, auth_token: e.target.value || null })
              }
              placeholder="(unset — uses provider env vars)"
              className="pr-16"
            />
            <button
              type="button"
              onClick={() => setTokenVisible((v) => !v)}
              className="absolute inset-y-0 right-0 px-2 text-[0.7rem] font-medium text-neutral-400 hover:text-neutral-100"
            >
              {tokenVisible ? "Hide" : "Show"}
            </button>
          </div>
        </label>
      </div>
      <div className="mt-4 flex justify-end gap-2">
        <Button
          variant="ghost"
          disabled={!dirty || isSaving}
          onClick={() => {
            setDraft(cfg);
            setTokenVisible(false);
            onDirtyChange(false);
          }}
        >
          Reset
        </Button>
        <Button
          variant="primary"
          disabled={!dirty || isSaving}
          onClick={async () => {
            await onSave(draft);
            setSaved(true);
          }}
        >
          {isSaving ? "Saving…" : "Save"}
        </Button>
      </div>
    </Card>
  );
}
