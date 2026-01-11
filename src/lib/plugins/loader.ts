// src/lib/plugins/loader.ts

import { readdir, readFile, access, stat } from "fs/promises";
import { join } from "path";
import { homedir } from "os";
import { LoadedPlugin, PluginManifest } from "./types";

const PLUGINS_DIR = join(homedir(), ".bot-hq", "plugins");

export async function getPluginsDirectory(): Promise<string> {
  return PLUGINS_DIR;
}

export async function ensurePluginsDirectory(): Promise<void> {
  const { mkdir } = await import("fs/promises");
  await mkdir(PLUGINS_DIR, { recursive: true });
}

export async function loadPluginManifest(pluginPath: string): Promise<PluginManifest | null> {
  const manifestPath = join(pluginPath, "plugin.json");

  try {
    await access(manifestPath);
    const content = await readFile(manifestPath, "utf-8");
    const manifest = JSON.parse(content) as PluginManifest;

    // Basic validation
    if (!manifest.name || !manifest.version) {
      console.error(`Invalid manifest at ${manifestPath}: missing name or version`);
      return null;
    }

    return manifest;
  } catch (error) {
    console.error(`Failed to load manifest from ${pluginPath}:`, error);
    return null;
  }
}

export async function discoverPlugins(): Promise<LoadedPlugin[]> {
  await ensurePluginsDirectory();

  const plugins: LoadedPlugin[] = [];

  try {
    const entries = await readdir(PLUGINS_DIR, { withFileTypes: true });

    for (const entry of entries) {
      if (!entry.isDirectory()) continue;

      const pluginPath = join(PLUGINS_DIR, entry.name);
      const manifest = await loadPluginManifest(pluginPath);

      if (manifest) {
        plugins.push({
          name: manifest.name,
          version: manifest.version,
          path: pluginPath,
          manifest,
          enabled: true, // Default to enabled, will be updated from DB
        });
      }
    }
  } catch (error) {
    console.error("Failed to discover plugins:", error);
  }

  return plugins;
}

export async function validateManifest(manifest: PluginManifest): Promise<string[]> {
  const errors: string[] = [];

  if (!manifest.name) {
    errors.push("Missing required field: name");
  }

  if (!manifest.version) {
    errors.push("Missing required field: version");
  }

  if (manifest.name && !/^[a-z0-9-]+$/.test(manifest.name)) {
    errors.push("Plugin name must be lowercase alphanumeric with hyphens only");
  }

  if (manifest.mcp) {
    if (!manifest.mcp.entry) {
      errors.push("MCP config missing entry point");
    }
    if (manifest.mcp.transport && manifest.mcp.transport !== "stdio") {
      errors.push("Only stdio transport is supported");
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
