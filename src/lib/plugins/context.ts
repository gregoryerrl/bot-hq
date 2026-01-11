// src/lib/plugins/context.ts

import { PluginContext, LoadedPlugin } from "./types";
import { PluginDataStore } from "./store";
import { getPluginRegistry } from "./registry";
import { getMcpManager } from "./mcp-manager";

export function createPluginContext(plugin: LoadedPlugin): PluginContext {
  if (!plugin.dbId) {
    throw new Error(`Plugin ${plugin.name} is not registered in database`);
  }

  const store = new PluginDataStore(plugin.dbId);
  const registry = getPluginRegistry();

  return {
    mcp: {
      call: async (tool: string, params: Record<string, unknown>) => {
        const manager = getMcpManager();
        return manager.callTool(plugin.name, tool, params);
      },
    },

    store: {
      get: (key: string) => store.get(key),
      set: (key: string, value: unknown) => store.set(key, value),
      delete: (key: string) => store.delete(key),
    },

    workspaceData: {
      get: (workspaceId: number) => store.getWorkspaceData(workspaceId),
      set: (workspaceId: number, data: unknown) => store.setWorkspaceData(workspaceId, data),
    },

    taskData: {
      get: (taskId: number) => store.getTaskData(taskId),
      set: (taskId: number, data: unknown) => store.setTaskData(taskId, data),
    },

    get settings() {
      // Lazy load settings
      return registry.getSettings(plugin.name) as unknown as Record<string, unknown>;
    },

    get credentials() {
      // Lazy load credentials
      return registry.getCredentials(plugin.name) as unknown as Record<string, string>;
    },

    log: {
      info: (msg: string) => console.log(`[${plugin.name}] INFO:`, msg),
      warn: (msg: string) => console.warn(`[${plugin.name}] WARN:`, msg),
      error: (msg: string) => console.error(`[${plugin.name}] ERROR:`, msg),
    },
  };
}
