// src/lib/plugins/index.ts

// Types
export * from "./types";

// Core
export { getPluginRegistry, initializePlugins } from "./registry";
export { discoverPlugins, loadPluginManifest, getPluginsDirectory } from "./loader";
export { getMcpManager } from "./mcp-manager";
export { createPluginContext } from "./context";
export { PluginDataStore } from "./store";

// Events
export {
  getPluginEvents,
  fireTaskCreated,
  fireTaskUpdated,
  fireAgentStart,
  fireAgentComplete,
  fireApprovalCreated,
  fireApprovalAccepted,
  fireApprovalRejected,
} from "./events";
