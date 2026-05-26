import { Card, CardDescription, CardTitle } from "../components/ui/Card";

/**
 * The Rust-side plugin runtime is scaffolded (`src/plugins/{manifest,loader,
 * capabilities,heartbeat}.rs`) but live install + enable/disable Tauri
 * commands aren't wired yet. This page documents the manifest schema so a
 * dev can drop a plugin into `<data_dir>/plugins/<id>/` and see it on next
 * restart of bot-hq once the install commands land.
 */
export function PluginManager() {
  return (
    <div className="mx-auto h-full max-w-3xl overflow-auto px-6 py-6">
      <div className="mb-6">
        <h1 className="text-xl font-semibold tracking-tight">Plugins</h1>
        <p className="mt-1 text-sm text-neutral-400">
          Plugin loader is scaffolded; live install commands land in a
          follow-up batch. Plugins live at{" "}
          <code className="rounded bg-elevated px-1 py-0.5 font-mono text-[0.78rem] text-neutral-200">
            ~/.bot-hq/plugins/&lt;id&gt;/
          </code>{" "}
          with a top-level{" "}
          <code className="rounded bg-elevated px-1 py-0.5 font-mono text-[0.78rem] text-neutral-200">
            manifest.json
          </code>
          .
        </p>
      </div>

      <Card className="mb-4 bg-surface">
        <CardTitle>No live plugins yet</CardTitle>
        <CardDescription>
          The Rust runtime can load + capability-gate manifests; the frontend
          install flow (Tauri command + enable/disable toggle) is the next
          shipping step. Until then, plugins drop in via the filesystem.
        </CardDescription>
      </Card>

      <Card className="bg-surface">
        <CardTitle>Manifest schema (v1)</CardTitle>
        <pre className="mt-3 overflow-x-auto rounded border border-default bg-canvas px-3 py-2 font-mono text-[0.75rem] leading-relaxed text-neutral-200">
          {`{
  "id": "discord",
  "name": "Discord Bridge",
  "version": "0.1.0",
  "entry": "index.html",
  "requested_capabilities": [
    "cl_index_search",
    "session_doc_search"
  ],
  "slots": [
    { "slot_name": "sidebar.bottom", "panel_route": null },
    { "slot_name": null, "panel_route": "/plugins/discord" }
  ]
}`}
        </pre>
        <div className="mt-3 space-y-2 text-xs text-neutral-400">
          <p>
            <b className="text-neutral-200">id</b> — lowercase alphanumeric +{" "}
            <code className="font-mono">-</code>; doubles as the iframe origin
            host (<code className="font-mono">plugin-&lt;id&gt;.localhost</code>
            ).
          </p>
          <p>
            <b className="text-neutral-200">requested_capabilities</b> —
            command names the plugin will invoke; each maps to{" "}
            <code className="font-mono">allow-&lt;kebab-cmd&gt;</code> in the
            generated Tauri capability JSON.
          </p>
          <p>
            <b className="text-neutral-200">slots</b> — UI contribution
            points. Either <code className="font-mono">slot_name</code> (host
            slot the iframe renders into) or{" "}
            <code className="font-mono">panel_route</code> (top-level route).
          </p>
        </div>
      </Card>
    </div>
  );
}
