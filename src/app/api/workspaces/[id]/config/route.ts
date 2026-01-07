import { NextRequest, NextResponse } from "next/server";
import { db, workspaces } from "@/lib/db";
import { eq } from "drizzle-orm";
import { parseAgentConfig, serializeAgentConfig, AgentConfig } from "@/lib/agents/config-types";

export async function GET(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const workspace = await db
      .select()
      .from(workspaces)
      .where(eq(workspaces.id, parseInt(id)))
      .limit(1);

    if (workspace.length === 0) {
      return NextResponse.json({ error: "Workspace not found" }, { status: 404 });
    }

    const config = parseAgentConfig(workspace[0].agentConfig);
    return NextResponse.json(config);
  } catch (error) {
    console.error("Failed to fetch workspace config:", error);
    return NextResponse.json({ error: "Failed to fetch config" }, { status: 500 });
  }
}

export async function PUT(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const body: Partial<AgentConfig> = await request.json();

    const workspace = await db
      .select()
      .from(workspaces)
      .where(eq(workspaces.id, parseInt(id)))
      .limit(1);

    if (workspace.length === 0) {
      return NextResponse.json({ error: "Workspace not found" }, { status: 404 });
    }

    const existingConfig = parseAgentConfig(workspace[0].agentConfig);
    const updatedConfig: AgentConfig = { ...existingConfig, ...body };

    const result = await db
      .update(workspaces)
      .set({ agentConfig: serializeAgentConfig(updatedConfig) })
      .where(eq(workspaces.id, parseInt(id)))
      .returning();

    return NextResponse.json(parseAgentConfig(result[0].agentConfig));
  } catch (error) {
    console.error("Failed to update config:", error);
    return NextResponse.json({ error: "Failed to update config" }, { status: 500 });
  }
}
