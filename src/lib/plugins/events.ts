// src/lib/plugins/events.ts

import { getPluginRegistry } from "./registry";
import { createPluginContext } from "./context";
import {
  PluginHooks,
  PluginExtensions,
  TaskHookData,
  AgentHookData,
  ApprovalHookData,
} from "./types";

type HookName = keyof PluginHooks;

class PluginEvents {
  private extensionsCache: Map<string, PluginExtensions> = new Map();

  private async loadExtensions(pluginName: string): Promise<PluginExtensions | null> {
    if (this.extensionsCache.has(pluginName)) {
      return this.extensionsCache.get(pluginName)!;
    }

    const registry = getPluginRegistry();
    const plugin = registry.getPlugin(pluginName);

    if (!plugin || !plugin.manifest.extensions) {
      return null;
    }

    try {
      // Dynamic import of extensions module
      const extensionsPath = `${plugin.path}/${plugin.manifest.extensions}`;
      const module = await import(extensionsPath);
      const context = createPluginContext(plugin);
      const extensions = module.default(context) as PluginExtensions;

      this.extensionsCache.set(pluginName, extensions);
      return extensions;
    } catch (error) {
      console.error(`Failed to load extensions for ${pluginName}:`, error);
      return null;
    }
  }

  async fireHook<T extends HookName>(
    hookName: T,
    ...args: Parameters<NonNullable<PluginHooks[T]>>
  ): Promise<Map<string, unknown>> {
    const results = new Map<string, unknown>();
    const registry = getPluginRegistry();
    const enabledPlugins = registry.getEnabledPlugins();

    await Promise.all(
      enabledPlugins.map(async (plugin) => {
        try {
          const extensions = await this.loadExtensions(plugin.name);
          const hook = extensions?.hooks?.[hookName];

          if (hook) {
            // eslint-disable-next-line @typescript-eslint/no-explicit-any
            const result = await (hook as any)(...args);
            results.set(plugin.name, result);
          }
        } catch (error) {
          console.error(`Hook ${hookName} failed for plugin ${plugin.name}:`, error);
        }
      })
    );

    return results;
  }

  async getApprovalActions(): Promise<Array<{
    pluginName: string;
    action: NonNullable<NonNullable<PluginExtensions["actions"]>["approval"]>[number];
  }>> {
    const actions: Array<{
      pluginName: string;
      action: NonNullable<NonNullable<PluginExtensions["actions"]>["approval"]>[number];
    }> = [];

    const registry = getPluginRegistry();
    const enabledPlugins = registry.getEnabledPlugins();

    for (const plugin of enabledPlugins) {
      const extensions = await this.loadExtensions(plugin.name);
      const pluginActions = extensions?.actions?.approval || [];

      for (const action of pluginActions) {
        actions.push({ pluginName: plugin.name, action });
      }
    }

    return actions;
  }

  async getTaskActions(): Promise<Array<{
    pluginName: string;
    action: NonNullable<NonNullable<PluginExtensions["actions"]>["task"]>[number];
  }>> {
    const actions: Array<{
      pluginName: string;
      action: NonNullable<NonNullable<PluginExtensions["actions"]>["task"]>[number];
    }> = [];

    const registry = getPluginRegistry();
    const enabledPlugins = registry.getEnabledPlugins();

    for (const plugin of enabledPlugins) {
      const extensions = await this.loadExtensions(plugin.name);
      const pluginActions = extensions?.actions?.task || [];

      for (const action of pluginActions) {
        actions.push({ pluginName: plugin.name, action });
      }
    }

    return actions;
  }

  clearCache(): void {
    this.extensionsCache.clear();
  }
}

// Singleton instance
let eventsInstance: PluginEvents | null = null;

export function getPluginEvents(): PluginEvents {
  if (!eventsInstance) {
    eventsInstance = new PluginEvents();
  }
  return eventsInstance;
}

// Convenience functions for firing specific hooks
export async function fireTaskCreated(task: TaskHookData): Promise<void> {
  await getPluginEvents().fireHook("onTaskCreated", task);
}

export async function fireTaskUpdated(task: TaskHookData, changes: Partial<TaskHookData>): Promise<void> {
  await getPluginEvents().fireHook("onTaskUpdated", task, changes);
}

export async function fireAgentStart(agent: AgentHookData, task: TaskHookData): Promise<Map<string, unknown>> {
  return getPluginEvents().fireHook("onAgentStart", agent, task);
}

export async function fireAgentComplete(agent: AgentHookData, task: TaskHookData): Promise<void> {
  await getPluginEvents().fireHook("onAgentComplete", agent, task);
}

export async function fireApprovalCreated(approval: ApprovalHookData): Promise<void> {
  await getPluginEvents().fireHook("onApprovalCreated", approval);
}

export async function fireApprovalAccepted(approval: ApprovalHookData, task: TaskHookData): Promise<void> {
  await getPluginEvents().fireHook("onApprovalAccepted", approval, task);
}

export async function fireApprovalRejected(approval: ApprovalHookData, task: TaskHookData): Promise<void> {
  await getPluginEvents().fireHook("onApprovalRejected", approval, task);
}
