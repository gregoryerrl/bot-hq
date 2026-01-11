// src/lib/plugins/registry.ts

import { eq } from "drizzle-orm";
import { db } from "@/lib/db";
import { plugins } from "@/lib/db/schema";
import { LoadedPlugin } from "./types";
import { discoverPlugins, validateManifest } from "./loader";

class PluginRegistry {
  private plugins: Map<string, LoadedPlugin> = new Map();
  private initialized = false;

  async initialize(): Promise<void> {
    if (this.initialized) return;

    const discovered = await discoverPlugins();
    
    for (const plugin of discovered) {
      // Validate manifest
      const errors = await validateManifest(plugin.manifest);
      if (errors.length > 0) {
        console.error(`Plugin ${plugin.name} has invalid manifest:`, errors);
        continue;
      }

      // Check if plugin exists in database
      const existing = await db
        .select()
        .from(plugins)
        .where(eq(plugins.name, plugin.name))
        .get();

      if (existing) {
        // Update from database
        plugin.dbId = existing.id;
        plugin.enabled = existing.enabled;

        // Update manifest if version changed
        if (existing.version !== plugin.version) {
          await db
            .update(plugins)
            .set({
              version: plugin.version,
              manifest: JSON.stringify(plugin.manifest),
              updatedAt: new Date(),
            })
            .where(eq(plugins.id, existing.id));
        }
      } else {
        // Insert new plugin
        const result = await db
          .insert(plugins)
          .values({
            name: plugin.name,
            version: plugin.version,
            manifest: JSON.stringify(plugin.manifest),
            enabled: true,
          })
          .returning({ id: plugins.id });

        plugin.dbId = result[0].id;
      }

      this.plugins.set(plugin.name, plugin);
    }

    this.initialized = true;
    console.log(`Plugin registry initialized with ${this.plugins.size} plugins`);
  }

  getPlugin(name: string): LoadedPlugin | undefined {
    return this.plugins.get(name);
  }

  getAllPlugins(): LoadedPlugin[] {
    return Array.from(this.plugins.values());
  }

  getEnabledPlugins(): LoadedPlugin[] {
    return this.getAllPlugins().filter(p => p.enabled);
  }

  async setEnabled(name: string, enabled: boolean): Promise<boolean> {
    const plugin = this.plugins.get(name);
    if (!plugin || !plugin.dbId) return false;

        await db
      .update(plugins)
      .set({ enabled, updatedAt: new Date() })
      .where(eq(plugins.id, plugin.dbId));

    plugin.enabled = enabled;
    return true;
  }

  async updateSettings(name: string, settings: Record<string, unknown>): Promise<boolean> {
    const plugin = this.plugins.get(name);
    if (!plugin || !plugin.dbId) return false;

        await db
      .update(plugins)
      .set({ settings: JSON.stringify(settings), updatedAt: new Date() })
      .where(eq(plugins.id, plugin.dbId));

    return true;
  }

  async getSettings(name: string): Promise<Record<string, unknown>> {
    const plugin = this.plugins.get(name);
    if (!plugin || !plugin.dbId) return {};

        const result = await db
      .select({ settings: plugins.settings })
      .from(plugins)
      .where(eq(plugins.id, plugin.dbId))
      .get();

    return result ? JSON.parse(result.settings) : {};
  }

  async updateCredentials(name: string, credentials: Record<string, string>): Promise<boolean> {
    const plugin = this.plugins.get(name);
    if (!plugin || !plugin.dbId) return false;

    // TODO: Encrypt credentials before storing
        await db
      .update(plugins)
      .set({ credentials: JSON.stringify(credentials), updatedAt: new Date() })
      .where(eq(plugins.id, plugin.dbId));

    return true;
  }

  async getCredentials(name: string): Promise<Record<string, string>> {
    const plugin = this.plugins.get(name);
    if (!plugin || !plugin.dbId) return {};

        const result = await db
      .select({ credentials: plugins.credentials })
      .from(plugins)
      .where(eq(plugins.id, plugin.dbId))
      .get();

    // TODO: Decrypt credentials
    return result?.credentials ? JSON.parse(result.credentials) : {};
  }

  isInitialized(): boolean {
    return this.initialized;
  }
}

// Singleton instance
let registryInstance: PluginRegistry | null = null;

export function getPluginRegistry(): PluginRegistry {
  if (!registryInstance) {
    registryInstance = new PluginRegistry();
  }
  return registryInstance;
}

export async function initializePlugins(): Promise<void> {
  const registry = getPluginRegistry();
  await registry.initialize();
}
