// src/lib/plugins/store.ts

import { eq, and } from "drizzle-orm";
import { db } from "@/lib/db";
import {
  pluginStore,
  pluginWorkspaceData,
  pluginTaskData,
} from "@/lib/db/schema";

export class PluginDataStore {
  constructor(private pluginId: number) {}

  // Global key-value store
  async get(key: string): Promise<unknown> {
    const result = await db
      .select({ value: pluginStore.value })
      .from(pluginStore)
      .where(
        and(
          eq(pluginStore.pluginId, this.pluginId),
          eq(pluginStore.key, key)
        )
      )
      .get();

    return result?.value ? JSON.parse(result.value) : undefined;
  }

  async set(key: string, value: unknown): Promise<void> {
    const serialized = JSON.stringify(value);

    // Upsert
    const existing = await db
      .select({ id: pluginStore.id })
      .from(pluginStore)
      .where(
        and(
          eq(pluginStore.pluginId, this.pluginId),
          eq(pluginStore.key, key)
        )
      )
      .get();

    if (existing) {
      await db
        .update(pluginStore)
        .set({ value: serialized, updatedAt: new Date() })
        .where(eq(pluginStore.id, existing.id));
    } else {
      await db.insert(pluginStore).values({
        pluginId: this.pluginId,
        key,
        value: serialized,
      });
    }
  }

  async delete(key: string): Promise<void> {
    await db
      .delete(pluginStore)
      .where(
        and(
          eq(pluginStore.pluginId, this.pluginId),
          eq(pluginStore.key, key)
        )
      );
  }

  // Workspace-scoped data
  async getWorkspaceData(workspaceId: number): Promise<unknown> {
    const result = await db
      .select({ data: pluginWorkspaceData.data })
      .from(pluginWorkspaceData)
      .where(
        and(
          eq(pluginWorkspaceData.pluginId, this.pluginId),
          eq(pluginWorkspaceData.workspaceId, workspaceId)
        )
      )
      .get();

    return result?.data ? JSON.parse(result.data) : undefined;
  }

  async setWorkspaceData(workspaceId: number, data: unknown): Promise<void> {
    const serialized = JSON.stringify(data);

    const existing = await db
      .select({ id: pluginWorkspaceData.id })
      .from(pluginWorkspaceData)
      .where(
        and(
          eq(pluginWorkspaceData.pluginId, this.pluginId),
          eq(pluginWorkspaceData.workspaceId, workspaceId)
        )
      )
      .get();

    if (existing) {
      await db
        .update(pluginWorkspaceData)
        .set({ data: serialized, updatedAt: new Date() })
        .where(eq(pluginWorkspaceData.id, existing.id));
    } else {
      await db.insert(pluginWorkspaceData).values({
        pluginId: this.pluginId,
        workspaceId,
        data: serialized,
      });
    }
  }

  // Task-scoped data
  async getTaskData(taskId: number): Promise<unknown> {
    const result = await db
      .select({ data: pluginTaskData.data })
      .from(pluginTaskData)
      .where(
        and(
          eq(pluginTaskData.pluginId, this.pluginId),
          eq(pluginTaskData.taskId, taskId)
        )
      )
      .get();

    return result?.data ? JSON.parse(result.data) : undefined;
  }

  async setTaskData(taskId: number, data: unknown): Promise<void> {
    const serialized = JSON.stringify(data);

    const existing = await db
      .select({ id: pluginTaskData.id })
      .from(pluginTaskData)
      .where(
        and(
          eq(pluginTaskData.pluginId, this.pluginId),
          eq(pluginTaskData.taskId, taskId)
        )
      )
      .get();

    if (existing) {
      await db
        .update(pluginTaskData)
        .set({ data: serialized, updatedAt: new Date() })
        .where(eq(pluginTaskData.id, existing.id));
    } else {
      await db.insert(pluginTaskData).values({
        pluginId: this.pluginId,
        taskId,
        data: serialized,
      });
    }
  }
}
