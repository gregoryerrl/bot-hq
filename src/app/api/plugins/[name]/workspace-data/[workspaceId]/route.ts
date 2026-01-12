import { NextRequest, NextResponse } from "next/server";
import { db, pluginWorkspaceData, plugins } from "@/lib/db";
import { eq, and } from "drizzle-orm";

export async function GET(
  request: NextRequest,
  { params }: { params: Promise<{ name: string; workspaceId: string }> }
) {
  try {
    const { name, workspaceId } = await params;

    // Get plugin ID
    const plugin = await db.query.plugins.findFirst({
      where: eq(plugins.name, name),
    });

    if (!plugin) {
      return NextResponse.json({ error: "Plugin not found" }, { status: 404 });
    }

    // Get workspace data
    const data = await db.query.pluginWorkspaceData.findFirst({
      where: and(
        eq(pluginWorkspaceData.pluginId, plugin.id),
        eq(pluginWorkspaceData.workspaceId, parseInt(workspaceId))
      ),
    });

    return NextResponse.json(data ? JSON.parse(data.data) : {});
  } catch (error) {
    console.error("Failed to get plugin workspace data:", error);
    return NextResponse.json({ error: "Failed to get data" }, { status: 500 });
  }
}

export async function PUT(
  request: NextRequest,
  { params }: { params: Promise<{ name: string; workspaceId: string }> }
) {
  try {
    const { name, workspaceId } = await params;
    const body = await request.json();

    // Get plugin ID
    const plugin = await db.query.plugins.findFirst({
      where: eq(plugins.name, name),
    });

    if (!plugin) {
      return NextResponse.json({ error: "Plugin not found" }, { status: 404 });
    }

    const wid = parseInt(workspaceId);

    // Check if data exists
    const existing = await db.query.pluginWorkspaceData.findFirst({
      where: and(
        eq(pluginWorkspaceData.pluginId, plugin.id),
        eq(pluginWorkspaceData.workspaceId, wid)
      ),
    });

    if (existing) {
      // Update
      await db
        .update(pluginWorkspaceData)
        .set({ data: JSON.stringify(body), updatedAt: new Date() })
        .where(eq(pluginWorkspaceData.id, existing.id));
    } else {
      // Insert
      await db.insert(pluginWorkspaceData).values({
        pluginId: plugin.id,
        workspaceId: wid,
        data: JSON.stringify(body),
      });
    }

    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Failed to update plugin workspace data:", error);
    return NextResponse.json({ error: "Failed to update data" }, { status: 500 });
  }
}
