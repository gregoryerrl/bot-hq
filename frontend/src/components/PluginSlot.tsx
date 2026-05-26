import type { ReactNode } from "react";

interface PluginSlotProps {
  name: string;
  /** Fallback rendered when no plugin contributes to this slot. */
  fallback?: ReactNode;
}

/**
 * Host component for plugin slot extensions. Batch 3 scaffolded the Rust-side
 * plugin module; per-slot iframe rendering arrives once a live plugin ships.
 * For now this is a placeholder that renders the fallback if no plugins are
 * registered for `name`.
 */
export function PluginSlot({ name, fallback = null }: PluginSlotProps) {
  // TODO(Batch 5+): subscribe to plugin registry, render iframes whose
  // manifest contributes to `name`. Until live plugins exist, this is a
  // no-op slot — see src/plugins/ for the scaffold.
  void name;
  return <>{fallback}</>;
}
