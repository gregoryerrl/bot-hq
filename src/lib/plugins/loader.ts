// src/lib/plugins/loader.ts

import { readdir, readFile, access, stat } from "fs/promises";
import { join } from "path";
import { homedir } from "os";
import { LoadedPlugin, PluginManifest } from "./types";

const PLUGINS_DIR = join(homedir(), ".bot-hq", "plugins");

export interface PluginLoadError {
  pluginPath: string;
  error: string;
  details?: string;
}

// Track load errors for reporting
const loadErrors: PluginLoadError[] = [];

export function getLoadErrors(): PluginLoadError[] {
  return [...loadErrors];
}

export function clearLoadErrors(): void {
  loadErrors.length = 0;
}

export async function getPluginsDirectory(): Promise<string> {
  return PLUGINS_DIR;
}

export async function ensurePluginsDirectory(): Promise<void> {
  const { mkdir } = await import("fs/promises");
  await mkdir(PLUGINS_DIR, { recursive: true });
}

export async function loadPluginManifest(pluginPath: string): Promise<PluginManifest | null> {
  const manifestPath = join(pluginPath, "plugin.json");

  // Check if manifest exists
  try {
    await access(manifestPath);
  } catch {
    loadErrors.push({
      pluginPath,
      error: "Missing plugin.json",
      details: `Expected manifest at: ${manifestPath}`,
    });
    console.error(`[Plugin] Missing plugin.json in ${pluginPath}`);
    return null;
  }

  // Read manifest file
  let content: string;
  try {
    content = await readFile(manifestPath, "utf-8");
  } catch (error) {
    loadErrors.push({
      pluginPath,
      error: "Failed to read plugin.json",
      details: error instanceof Error ? error.message : "Unknown read error",
    });
    console.error(`[Plugin] Failed to read manifest from ${pluginPath}:`, error);
    return null;
  }

  // Parse JSON
  let manifest: PluginManifest;
  try {
    manifest = JSON.parse(content) as PluginManifest;
  } catch (error) {
    loadErrors.push({
      pluginPath,
      error: "Invalid JSON in plugin.json",
      details: error instanceof Error ? error.message : "JSON parse error",
    });
    console.error(`[Plugin] Invalid JSON in ${manifestPath}:`, error);
    return null;
  }

  // Validate required fields
  const validationErrors = await validateManifest(manifest);
  if (validationErrors.length > 0) {
    loadErrors.push({
      pluginPath,
      error: "Invalid manifest",
      details: validationErrors.join("; "),
    });
    console.error(`[Plugin] Invalid manifest in ${pluginPath}: ${validationErrors.join(", ")}`);
    return null;
  }

  return manifest;
}

export async function discoverPlugins(): Promise<LoadedPlugin[]> {
  // Clear previous errors
  clearLoadErrors();

  await ensurePluginsDirectory();

  const plugins: LoadedPlugin[] = [];

  let entries;
  try {
    entries = await readdir(PLUGINS_DIR, { withFileTypes: true });
  } catch (error) {
    console.error("[Plugin] Failed to read plugins directory:", error);
    return plugins;
  }

  for (const entry of entries) {
    // Skip hidden directories and non-directories
    if (!entry.isDirectory() || entry.name.startsWith(".")) {
      continue;
    }

    const pluginPath = join(PLUGINS_DIR, entry.name);

    try {
      const manifest = await loadPluginManifest(pluginPath);

      if (manifest) {
        plugins.push({
          name: manifest.name,
          version: manifest.version,
          path: pluginPath,
          manifest,
          enabled: true, // Default to enabled, will be updated from DB
        });
        console.log(`[Plugin] Loaded: ${manifest.name} v${manifest.version}`);
      }
    } catch (error) {
      // Catch any unexpected errors during plugin loading
      loadErrors.push({
        pluginPath,
        error: "Unexpected error loading plugin",
        details: error instanceof Error ? error.message : "Unknown error",
      });
      console.error(`[Plugin] Unexpected error loading ${pluginPath}:`, error);
    }
  }

  // Report summary
  if (loadErrors.length > 0) {
    console.warn(`[Plugin] ${loadErrors.length} plugin(s) failed to load`);
  }
  console.log(`[Plugin] Discovered ${plugins.length} valid plugin(s)`);

  return plugins;
}

export async function validateManifest(manifest: PluginManifest): Promise<string[]> {
  const errors: string[] = [];

  // Required fields
  if (!manifest.name) {
    errors.push("Missing required field: name");
  } else if (typeof manifest.name !== "string") {
    errors.push("Field 'name' must be a string");
  } else if (!/^[a-z0-9-]+$/.test(manifest.name)) {
    errors.push("Plugin name must be lowercase alphanumeric with hyphens only");
  }

  if (!manifest.version) {
    errors.push("Missing required field: version");
  } else if (typeof manifest.version !== "string") {
    errors.push("Field 'version' must be a string");
  }

  // MCP configuration validation
  if (manifest.mcp) {
    if (typeof manifest.mcp !== "object") {
      errors.push("Field 'mcp' must be an object");
    } else {
      if (!manifest.mcp.entry) {
        errors.push("MCP config missing required field: entry");
      }
      if (manifest.mcp.transport && manifest.mcp.transport !== "stdio") {
        errors.push("Only stdio transport is supported for MCP");
      }
    }
  }

  // UI configuration validation
  if (manifest.ui) {
    if (typeof manifest.ui !== "object") {
      errors.push("Field 'ui' must be an object");
    } else {
      if (manifest.ui.tabs && !Array.isArray(manifest.ui.tabs)) {
        errors.push("Field 'ui.tabs' must be an array");
      }
    }
  }

  // Settings validation
  if (manifest.settings) {
    if (typeof manifest.settings !== "object") {
      errors.push("Field 'settings' must be an object");
    }
  }

  // Credentials validation
  if (manifest.credentials) {
    if (typeof manifest.credentials !== "object") {
      errors.push("Field 'credentials' must be an object");
    }
  }

  return errors;
}

export async function pluginExists(name: string): Promise<boolean> {
  const pluginPath = join(PLUGINS_DIR, name);
  try {
    const stats = await stat(pluginPath);
    return stats.isDirectory();
  } catch {
    return false;
  }
}
