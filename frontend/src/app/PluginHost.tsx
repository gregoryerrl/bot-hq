import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTauriEvent } from "../hooks/useTauriEvent";
import { Button } from "../components/ui/Button";
import { Card, CardDescription, CardTitle } from "../components/ui/Card";
import { ConfirmDialog } from "../components/ConfirmDialog";
import {
  mountPluginBridge,
  pluginEntryUrl,
  schemeForm,
  type SpawnRequest,
} from "../lib/pluginBridge";
import type { InstalledPluginView } from "../lib/bindings";

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
              <pre className="max-h-48 overflow-auto whitespace-pre-wrap rounded bg-surface-container-high p-2 font-code-sm text-code-sm text-on-surface">
                {pendingSpawn.req.prompt}
              </pre>
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
