"use client";

import { use } from "react";
import { Header } from "@/components/layout/header";
import { GitHubPluginPage } from "@/components/plugins/github-plugin-page";

const PLUGIN_COMPONENTS: Record<string, Record<string, { title: string; description: string; component: React.ComponentType }>> = {
  github: {
    main: {
      title: "GitHub",
      description: "GitHub integration - clone repos, manage issues, and more",
      component: GitHubPluginPage,
    },
  },
};

export default function PluginTabPage({
  params,
}: {
  params: Promise<{ pluginName: string; tabId: string }>;
}) {
  const { pluginName, tabId } = use(params);

  const pluginConfig = PLUGIN_COMPONENTS[pluginName]?.[tabId];

  if (pluginConfig) {
    const Component = pluginConfig.component;
    return (
      <div className="flex flex-col h-full">
        <Header
          title={pluginConfig.title}
          description={pluginConfig.description}
        />
        <div className="flex-1 p-4 md:p-6 overflow-auto">
          <Component />
        </div>
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full">
      <Header
        title={`${pluginName} - ${tabId}`}
        description="Plugin contributed tab"
      />
      <div className="flex-1 p-6">
        <div className="bg-muted/30 rounded-lg border p-8 text-center">
          <p className="text-muted-foreground">
            Plugin tab content for <strong>{pluginName}/{tabId}</strong>
          </p>
          <p className="text-sm text-muted-foreground mt-2">
            Component not found for this plugin tab.
          </p>
        </div>
      </div>
    </div>
  );
}
