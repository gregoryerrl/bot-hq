import { NextResponse } from "next/server";
import { db, workspaces } from "@/lib/db";
import { eq } from "drizzle-orm";
import { getWorkspaceContext, saveWorkspaceContext, initializeWorkspaceContext } from "@/lib/bot-hq";

export async function GET(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const workspaceId = parseInt(id);

    const workspace = await db.query.workspaces.findFirst({
      where: eq(workspaces.id, workspaceId),
    });

    if (!workspace) {
      return NextResponse.json({ error: "Workspace not found" }, { status: 404 });
    }

    // Initialize if needed
    await initializeWorkspaceContext(workspace.name);

    const context = await getWorkspaceContext(workspace.name);

    return NextResponse.json({
      workspaceId,
      workspaceName: workspace.name,
      context,
    });
  } catch (error) {
    console.error("Failed to get workspace context:", error);
    return NextResponse.json(
      { error: "Failed to get workspace context" },
      { status: 500 }
    );
  }
}

export async function PUT(
  request: Request,
  { params }: { params: Promise<{ id: string }> }
) {
  try {
    const { id } = await params;
    const workspaceId = parseInt(id);
    const { context } = await request.json();

    const workspace = await db.query.workspaces.findFirst({
      where: eq(workspaces.id, workspaceId),
    });

    if (!workspace) {
      return NextResponse.json({ error: "Workspace not found" }, { status: 404 });
    }

    await saveWorkspaceContext(workspace.name, context);

    return NextResponse.json({ success: true });
  } catch (error) {
    console.error("Failed to save workspace context:", error);
    return NextResponse.json(
      { error: "Failed to save workspace context" },
      { status: 500 }
    );
  }
}
