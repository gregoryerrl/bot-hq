import { Card, CardDescription, CardTitle } from "../components/ui/Card";

export function PluginManager() {
  // Live plugins not yet wired (Batch 3 shipped the Rust-side scaffolding;
  // Batch 5+ wires real plugin loading via tauri_cmd::plugins). Surface a
  // placeholder so the UI route exists without lying about features.
  return (
    <div className="mx-auto max-w-4xl px-6 py-6">
      <h1 className="mb-4 text-xl font-semibold">Plugins</h1>
      <Card>
        <CardTitle>Plugin model is scaffolded; no installs yet</CardTitle>
        <CardDescription>
          Plugin runtime (manifest parser, capability JSON, iframe origin
          gating, heartbeat watcher) is in `src/plugins/`. Live plugin install +
          enable / disable wiring is the next milestone. Discord + Clive
          plugins are the first targets.
        </CardDescription>
      </Card>
    </div>
  );
}
