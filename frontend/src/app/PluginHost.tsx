import { useEffect, useMemo, useRef, useState } from "react";
import { useTauriEvent } from "../hooks/useTauriEvent";
import { Button } from "../components/ui/Button";
import { Card, CardDescription, CardTitle } from "../components/ui/Card";
import {
  mountPluginBridge,
  pluginEntryUrl,
  schemeForm,
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
  const iframeRef = useRef<HTMLIFrameElement>(null);

  const form = useMemo(() => schemeForm(), []);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const nonce = useMemo(() => crypto.randomUUID(), [epoch]);
  const src = pluginEntryUrl(form, plugin.id, plugin.manifest.entry, nonce);

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
    return mountPluginBridge({ iframe, pluginId: plugin.id, nonce, form });
  }, [plugin.id, nonce, form, crashed]);

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
  );
}
