import { Link, useParams } from "react-router-dom";
import { useTauriQuery } from "../hooks/useInvoke";
import { Card, CardDescription, CardTitle } from "../components/ui/Card";
import { PluginHost } from "./PluginHost";
import type { InstalledPluginView } from "../lib/bindings";

/** Full-page plugin panel behind `/plugins/view/:pluginId`. */
export function PluginPanel() {
  const { pluginId } = useParams<{ pluginId: string }>();
  const list = useTauriQuery<InstalledPluginView[]>("list_installed_plugins", {});

  if (list.isLoading) {
    return (
      <p className="p-6 font-body-md text-body-md text-on-surface-variant">
        Loading…
      </p>
    );
  }

  const plugin = (list.data ?? []).find((p) => p.id === pluginId);
  if (!plugin || !plugin.enabled) {
    return (
      <div className="flex h-full items-center justify-center p-6">
        <Card className="max-w-md bg-surface text-center">
          <CardTitle>
            {plugin ? `${plugin.name} is disabled` : "Plugin not found"}
          </CardTitle>
          <CardDescription>
            {plugin
              ? "Enable it in the Plugins tab to open its panel."
              : `No installed plugin with id "${pluginId}".`}{" "}
            <Link to="/plugins" className="text-tertiary underline">
              Open Plugins
            </Link>
          </CardDescription>
        </Card>
      </div>
    );
  }

  return <PluginHost plugin={plugin} />;
}
