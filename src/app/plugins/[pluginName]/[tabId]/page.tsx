"use client";

import { use } from "react";
import { Header } from "@/components/layout/header";

export default function PluginTabPage({
  params,
}: {
  params: Promise<{ pluginName: string; tabId: string }>;
}) {
  const { pluginName, tabId } = use(params);

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
            Plugin UI rendering will be implemented when plugins provide components.
          </p>
        </div>
      </div>
    </div>
  );
}
