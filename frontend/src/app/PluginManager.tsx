import { useState } from "react";
import { useTauriQuery, useTauriMutation } from "../hooks/useInvoke";
import { useTauriEvent } from "../hooks/useTauriEvent";
import { Button } from "../components/ui/Button";
import { Card, CardDescription, CardTitle } from "../components/ui/Card";
import { ConfirmDialog } from "../components/ConfirmDialog";
import { cn } from "../lib/cn";
import type {
  AppError,
  InstalledPluginView,
  PluginStatus,
} from "../lib/bindings";

/**
 * Live PluginManager: list installed plugins, install new ones (URL or
 * local path), enable/disable/uninstall, watch heartbeat status updates
 * via Tauri events.
 */
export function PluginManager() {
  const [installSource, setInstallSource] = useState("");
  const [installError, setInstallError] = useState<AppError | null>(null);
  // Enable/disable + uninstall both fire-and-forget mutate; capture their
  // rejections so a failed toggle/uninstall isn't silently swallowed.
  const [toggleError, setToggleError] = useState<AppError | null>(null);
  const [uninstallError, setUninstallError] = useState<AppError | null>(null);
  const [confirmUninstall, setConfirmUninstall] =
    useState<InstalledPluginView | null>(null);

  const list = useTauriQuery<InstalledPluginView[]>(
    "list_installed_plugins",
    {},
    { refetchInterval: 10_000 },
  );
  const plugins = list.data ?? [];

  const install = useTauriMutation<InstalledPluginView, { source: string }>(
    "install_plugin",
  );
  const enable = useTauriMutation<void, { pluginId: string }>("enable_plugin");
  const disable = useTauriMutation<void, { pluginId: string }>("disable_plugin");
  const uninstall = useTauriMutation<void, { pluginId: string }>(
    "uninstall_plugin",
  );

  // Refetch on any backend state change. The plugin:state-changed event
  // covers enable/disable; uninstall + crash carry their own events.
  useTauriEvent<{ plugin_id: string }>(
    "plugin:state-changed",
    () => void list.refetch(),
    [list.refetch],
  );
  useTauriEvent<{ plugin_id: string }>(
    "plugin:uninstalled",
    () => void list.refetch(),
    [list.refetch],
  );
  useTauriEvent<{ plugin_id: string }>(
    "plugin:crashed",
    () => void list.refetch(),
    [list.refetch],
  );

  const handleInstall = () => {
    const source = installSource.trim();
    if (!source || install.isPending) return;
    setInstallError(null);
    install.mutate(
      { source },
      {
        onSuccess: () => {
          setInstallSource("");
          void list.refetch();
        },
        onError: (err) => setInstallError(err),
      },
    );
  };

  return (
    <div className="mx-auto h-full max-w-3xl overflow-auto px-6 py-6">
      <header className="mb-6 flex items-baseline gap-3">
        <h1 className="text-xl font-semibold tracking-tight">Plugins</h1>
        <span className="text-xs text-on-surface-variant">
          {plugins.length} installed
        </span>
      </header>

      <section className="mb-6">
        <div className="flex gap-2">
          <input
            type="text"
            value={installSource}
            onChange={(e) => setInstallSource(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                handleInstall();
              }
            }}
            placeholder="URL to manifest.json or local directory path…"
            className="flex-1 rounded-md border border-outline-variant bg-surface-container-high px-3 py-1.5 text-sm text-on-surface placeholder:text-on-surface-variant focus:outline-none focus:ring-1 focus:ring-primary"
          />
          <Button
            variant="primary"
            onClick={handleInstall}
            disabled={!installSource.trim() || install.isPending}
          >
            {install.isPending ? "Installing…" : "Install"}
          </Button>
        </div>
        {installError && (
          <div className="mt-2 flex items-start justify-between gap-3 rounded border border-outline-variant bg-error-container/30 px-3 py-2 text-xs text-on-error-container">
            <div>
              <span className="font-semibold">{installError.kind}:</span>{" "}
              {installError.message}
            </div>
            <button
              className="underline"
              onClick={() => setInstallError(null)}
            >
              dismiss
            </button>
          </div>
        )}
        {toggleError && (
          <div className="mt-2 flex items-start justify-between gap-3 rounded border border-outline-variant bg-error-container/30 px-3 py-2 text-xs text-on-error-container">
            <div>
              <span className="font-semibold">{toggleError.kind}:</span>{" "}
              Enable/disable failed: {toggleError.message}
            </div>
            <button className="underline" onClick={() => setToggleError(null)}>
              dismiss
            </button>
          </div>
        )}
        {uninstallError && (
          <div className="mt-2 flex items-start justify-between gap-3 rounded border border-outline-variant bg-error-container/30 px-3 py-2 text-xs text-on-error-container">
            <div>
              <span className="font-semibold">{uninstallError.kind}:</span>{" "}
              Uninstall failed: {uninstallError.message}
            </div>
            <button
              className="underline"
              onClick={() => setUninstallError(null)}
            >
              dismiss
            </button>
          </div>
        )}
      </section>

      {list.isLoading ? (
        <p className="text-sm text-on-surface-variant">Loading…</p>
      ) : plugins.length === 0 ? (
        <Card className="bg-surface">
          <CardTitle>No plugins installed</CardTitle>
          <CardDescription>
            Paste a manifest URL or a local plugin directory above to install.
            Plugins live at{" "}
            <code className="rounded bg-surface-container-high px-1 py-0.5 font-mono text-[0.78rem] text-on-surface">
              ~/.bot-hq/plugins/&lt;id&gt;/
            </code>{" "}
            once installed.
          </CardDescription>
        </Card>
      ) : (
        <div className="space-y-3">
          {plugins.map((p) => (
            <PluginCard
              key={p.id}
              plugin={p}
              onToggle={() => {
                const action = p.enabled ? disable : enable;
                setToggleError(null);
                action.mutate(
                  { pluginId: p.id },
                  { onError: (err) => setToggleError(err) },
                );
              }}
              onUninstall={() => setConfirmUninstall(p)}
              busy={
                (p.enabled && disable.isPending) ||
                (!p.enabled && enable.isPending) ||
                uninstall.isPending
              }
            />
          ))}
        </div>
      )}
      <ConfirmDialog
        open={confirmUninstall !== null}
        title="Uninstall plugin?"
        message={
          <>
            Uninstall{" "}
            <strong className="text-on-surface">{confirmUninstall?.name}</strong>?
            Its files under{" "}
            <code className="text-on-surface">~/.bot-hq/plugins/</code> are
            removed.
          </>
        }
        confirmLabel="Uninstall"
        confirmVariant="danger"
        onConfirm={() => {
          if (confirmUninstall) {
            setUninstallError(null);
            uninstall.mutate(
              { pluginId: confirmUninstall.id },
              { onError: (err) => setUninstallError(err) },
            );
          }
          setConfirmUninstall(null);
        }}
        onCancel={() => setConfirmUninstall(null)}
      />
    </div>
  );
}

interface PluginCardProps {
  plugin: InstalledPluginView;
  onToggle: () => void;
  onUninstall: () => void;
  busy: boolean;
}

function PluginCard({ plugin, onToggle, onUninstall, busy }: PluginCardProps) {
  const { manifest, status, enabled } = plugin;
  const panelSlot = manifest.slots?.find((s) => s.panel_route);
  const namedSlots = (manifest.slots ?? []).filter((s) => s.slot_name);

  return (
    <Card className="bg-surface">
      <header className="mb-2 flex items-center gap-2">
        <span
          aria-hidden
          className={cn("size-2 rounded-full", statusDotClass(status, enabled))}
          title={statusLabel(status, enabled)}
        />
        <CardTitle>{plugin.name}</CardTitle>
        <span className="rounded bg-surface-container-high px-1.5 py-0.5 font-mono text-[0.65rem] text-on-surface">
          v{plugin.version}
        </span>
        <span className="ml-auto text-[0.65rem] text-on-surface-variant">
          {statusLabel(status, enabled)}
        </span>
      </header>

      <div className="mb-3 text-xs text-on-surface-variant">
        <code className="font-mono">{manifest.id}</code> · entry{" "}
        <code className="font-mono">{manifest.entry}</code>
        {manifest.requested_capabilities &&
          manifest.requested_capabilities.length > 0 && (
            <>
              {" "}
              · caps:{" "}
              {manifest.requested_capabilities.map((c) => (
                <code
                  key={c}
                  className="ml-1 rounded bg-surface-container-high px-1 py-0.5 font-mono text-[0.65rem] text-on-surface"
                >
                  {c}
                </code>
              ))}
            </>
          )}
      </div>

      {namedSlots.length > 0 && (
        <div className="mb-3 text-[0.65rem] text-on-surface-variant">
          slots:{" "}
          {namedSlots.map((s, i) => (
            <code
              key={i}
              className="ml-1 rounded bg-surface-container-high px-1 py-0.5 font-mono text-on-surface"
            >
              {s.slot_name}
            </code>
          ))}
        </div>
      )}

      <div className="flex flex-wrap items-center gap-2">
        <Button
          variant={enabled ? "secondary" : "primary"}
          size="sm"
          onClick={onToggle}
          disabled={busy}
        >
          {enabled ? "Disable" : "Enable"}
        </Button>
        <Button
          variant="danger"
          size="sm"
          onClick={onUninstall}
          disabled={busy}
        >
          Uninstall
        </Button>
        {panelSlot?.panel_route && (
          <a
            href={`#${panelSlot.panel_route}`}
            className="ml-auto text-xs text-tertiary underline hover:text-tertiary"
          >
            Open panel →
          </a>
        )}
      </div>
    </Card>
  );
}

function statusDotClass(status: PluginStatus, enabled: boolean): string {
  if (!enabled) return "bg-outline-variant";
  switch (status.kind) {
    case "Healthy":
      return "bg-success";
    case "Slow":
      return "animate-pulse bg-warning";
    case "Crashed":
      return "bg-error";
  }
}

function statusLabel(status: PluginStatus, enabled: boolean): string {
  if (!enabled) return "disabled";
  switch (status.kind) {
    case "Healthy":
      return "healthy";
    case "Slow":
      return `slow · ${status.miss_count} miss${status.miss_count === 1 ? "" : "es"}`;
    case "Crashed":
      return "crashed";
  }
}
