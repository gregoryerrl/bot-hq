import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTauriEvent } from "../hooks/useTauriEvent";
import { Button } from "../components/ui/Button";
import { Card, CardDescription, CardTitle } from "../components/ui/Card";
import { ConfirmDialog } from "../components/ConfirmDialog";
import {
  mountPluginBridge,
  pluginEntryUrl,
  postPluginEvent,
  schemeForm,
  type SpawnRequest,
} from "../lib/pluginBridge";
import type { InstalledPluginView } from "../lib/bindings";

/**
 * Advisory heuristic for the spawn confirm dialog: a prompt whose last
 * non-empty line ends with ":" looks like it ends with an unfilled
 * template section (the failure that shipped a spawn with an empty
 * "Task:" tail). Advisory only — legitimate prompts can end with a
 * colon, so the dialog warns and never blocks.
 */
export function promptEndsWithUnfilledSection(prompt: string): boolean {
  const lines = prompt.split("\n");
  for (let i = lines.length - 1; i >= 0; i--) {
    const line = lines[i].trim();
    if (line) return line.endsWith(":");
  }
  return false;
}

/**
 * The live plugin surface: one sandboxed iframe per mount, wired to the
 * postMessage RPC bridge + heartbeat. When the backend sweep declares the
 * plugin crashed (`plugin:crashed`), the iframe is torn down and replaced
 * with a fallback card; Reload remounts under a fresh nonce.
 */
export function PluginHost({ plugin }: { plugin: InstalledPluginView }) {
  // Bumping the epoch remounts the iframe (new key) under a new nonce.
  const [epoch, setEpoch] = useState(0);
  const [crashed, setCrashed] = useState(false);
  // The per-spawn confirm (spawn_session tier): the bridge hands us the
  // request + a resolver; the dialog's buttons settle it. Anything running
  // inside the plugin's origin — panel code OR a user-commissioned
  // material's script — goes through this human gate.
  const [pendingSpawn, setPendingSpawn] = useState<{
    req: SpawnRequest;
    resolve: (ok: boolean) => void;
  } | null>(null);
  const iframeRef = useRef<HTMLIFrameElement>(null);

  const form = useMemo(() => schemeForm(), []);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const nonce = useMemo(() => crypto.randomUUID(), [epoch]);
  const src = pluginEntryUrl(form, plugin.id, plugin.manifest.entry, nonce);

  const confirmSpawn = useCallback(
    (req: SpawnRequest) =>
      new Promise<boolean>((resolve) => setPendingSpawn({ req, resolve })),
    [],
  );
  const spawnGranted =
    plugin.enabled &&
    (plugin.manifest.requested_capabilities ?? []).includes("spawn_session");

  useTauriEvent<{ plugin_id: string }>(
    "plugin:crashed",
    (payload) => {
      if (payload.plugin_id === plugin.id) setCrashed(true);
    },
    [plugin.id],
  );

  // Push tier (v1, two topics). assets_changed: the plugin's OWN served dir
  // changed on disk — no grant needed. sessions_changed: rides the
  // list_sessions grant (a plugin that can't read the list has no use for,
  // and no right to, the change signal).
  useTauriEvent<{ plugin_id: string }>(
    "plugin:assets_changed",
    (payload) => {
      const iframe = iframeRef.current;
      if (iframe && payload.plugin_id === plugin.id) {
        postPluginEvent(iframe, "plugin_assets_changed");
      }
    },
    [plugin.id],
  );
  const sessionsGranted = (plugin.manifest.requested_capabilities ?? []).includes(
    "list_sessions",
  );
  useTauriEvent<{ session_id: string }>(
    "session:created",
    () => {
      const iframe = iframeRef.current;
      if (iframe && sessionsGranted) postPluginEvent(iframe, "sessions_changed");
    },
    [sessionsGranted],
  );
  useTauriEvent<{ session_id: string }>(
    "session:closed",
    () => {
      const iframe = iframeRef.current;
      if (iframe && sessionsGranted) postPluginEvent(iframe, "sessions_changed");
    },
    [sessionsGranted],
  );

  useEffect(() => {
    const iframe = iframeRef.current;
    if (!iframe || crashed) return;
    return mountPluginBridge({
      iframe,
      pluginId: plugin.id,
      nonce,
      form,
      spawn: { granted: spawnGranted, confirm: confirmSpawn },
    });
  }, [plugin.id, nonce, form, crashed, spawnGranted, confirmSpawn]);

  if (crashed) {
    return (
      <div className="flex h-full items-center justify-center p-6">
        <Card className="max-w-md bg-surface text-center">
          <CardTitle>{plugin.name} stopped responding</CardTitle>
          <CardDescription>
            The plugin missed its health checks and was shut down. Its files
            are untouched — reload to try again.
          </CardDescription>
          <div className="mt-4">
            <Button
              variant="primary"
              onClick={() => {
                setCrashed(false);
                setEpoch((e) => e + 1);
              }}
            >
              Reload plugin
            </Button>
          </div>
        </Card>
      </div>
    );
  }

  return (
    <>
      <iframe
        key={epoch}
        ref={iframeRef}
        src={src}
        // allow-same-origin keeps the plugin's own origin (so its fetches +
        // storage work); it is NOT the host origin, so this does not weaken
        // the boundary. No allow-top-navigation / popups / forms.
        sandbox="allow-scripts allow-same-origin"
        className="h-full w-full border-0 bg-surface"
        title={plugin.name}
      />
      <ConfirmDialog
        open={pendingSpawn !== null}
        title={`Allow ${plugin.name} to open a session?`}
        message={
          pendingSpawn && (
            <div className="text-left">
              <p className="mb-2">
                Target project:{" "}
                <code className="font-code-sm">
                  {pendingSpawn.req.project ?? "(none — general)"}
                </code>
              </p>
              <p className="mb-1">
                It will start a new agent session with this prompt:
              </p>
              <pre className="max-h-72 overflow-y-auto overflow-x-hidden whitespace-pre-wrap rounded bg-surface-container-high p-2 font-code-sm text-code-sm text-on-surface">
                {pendingSpawn.req.prompt}
              </pre>
              {/* ~20 lines fit in max-h-72; past that the tail is below the
                  fold, so signpost it — the empty-tail incident was approved
                  because nothing hinted there was more to see. */}
              {pendingSpawn.req.prompt.split("\n").length > 20 && (
                <p className="mt-1 font-code-sm text-code-sm text-on-surface-variant">
                  {pendingSpawn.req.prompt.split("\n").length}-line prompt —
                  scroll the box to review it fully.
                </p>
              )}
              {promptEndsWithUnfilledSection(pendingSpawn.req.prompt) && (
                <p className="mt-2 rounded border border-warning/40 bg-warning/10 px-2 py-1 text-on-surface">
                  The prompt appears to end with an unfilled section (its
                  last line ends with ":"). Review the tail before approving.
                </p>
              )}
            </div>
          )
        }
        confirmLabel="Open session"
        confirmVariant="primary"
        onConfirm={() => {
          pendingSpawn?.resolve(true);
          setPendingSpawn(null);
        }}
        onCancel={() => {
          pendingSpawn?.resolve(false);
          setPendingSpawn(null);
        }}
      />
    </>
  );
}
