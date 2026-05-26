import type { ReactNode } from "react";
import { useTauriQuery } from "../hooks/useInvoke";
import type { InstalledPluginView } from "../lib/bindings";

interface PluginSlotProps {
  name: string;
  /** Fallback rendered when no enabled plugin contributes to this slot. */
  fallback?: ReactNode;
}

/**
 * Host slot for plugin extensions. Queries the installed-plugin list (shared
 * cache key with PluginManager) and renders an iframe per enabled plugin
 * whose manifest declares a matching `slot_name`. The iframe origin is
 * `https://plugin-<id>.localhost`, which Tauri's per-plugin capability JSON
 * scopes to the requested permissions.
 */
export function PluginSlot({ name, fallback = null }: PluginSlotProps) {
  const { data: plugins = [] } = useTauriQuery<InstalledPluginView[]>(
    "list_installed_plugins",
    {},
    { refetchInterval: 30_000 },
  );

  const contributors = plugins.filter(
    (p) =>
      p.enabled &&
      p.status.kind !== "Crashed" &&
      (p.manifest.slots ?? []).some((s) => s.slot_name === name),
  );

  if (contributors.length === 0) return <>{fallback}</>;

  return (
    <>
      {contributors.map((p) => (
        <iframe
          key={p.id}
          src={`https://plugin-${p.id}.localhost`}
          className="h-full w-full border-0"
          title={p.name}
          sandbox="allow-scripts allow-same-origin"
        />
      ))}
    </>
  );
}
