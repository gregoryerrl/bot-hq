import { NextResponse } from "next/server";
import { getPluginRegistry } from "@/lib/plugins";

interface PluginUIContribution {
  pluginName: string;
  tabs?: { id: string; label: string; icon: string; component: string }[];
  workspaceSettings?: string;
  taskBadge?: string;
  taskActions?: string;
}

export async function GET() {
  try {
    const registry = getPluginRegistry();
    const plugins = registry.getEnabledPlugins();

    const contributions: PluginUIContribution[] = [];

    for (const plugin of plugins) {
      if (plugin.manifest.ui) {
        contributions.push({
          pluginName: plugin.name,
          tabs: plugin.manifest.ui.tabs,
          workspaceSettings: plugin.manifest.ui.workspaceSettings,
          taskBadge: plugin.manifest.ui.taskBadge,
          taskActions: plugin.manifest.ui.taskActions,
        });
      }
    }

    return NextResponse.json(contributions);
  } catch (error) {
    console.error("Failed to get plugin UI contributions:", error);
    return NextResponse.json({ error: "Failed to get UI contributions" }, { status: 500 });
  }
}
