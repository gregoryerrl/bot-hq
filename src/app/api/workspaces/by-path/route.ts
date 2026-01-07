import { NextRequest, NextResponse } from "next/server";
import { db, workspaces } from "@/lib/db";
import { parseAgentConfig } from "@/lib/agents/config-types";

export async function GET(request: NextRequest) {
  try {
    const repoPath = request.nextUrl.searchParams.get("path");

    if (!repoPath) {
      return NextResponse.json({ error: "Path required" }, { status: 400 });
    }

    const normalizedPath = repoPath.replace("~", process.env.HOME || "");
    const allWorkspaces = await db.select().from(workspaces);

    const workspace = allWorkspaces.find(w => {
      const wsPath = w.repoPath.replace("~", process.env.HOME || "");
      return normalizedPath === wsPath || normalizedPath.startsWith(wsPath + "/");
    });

    if (!workspace) {
      return NextResponse.json({ error: "Workspace not found" }, { status: 404 });
    }

    return NextResponse.json({
      ...workspace,
      config: parseAgentConfig(workspace.agentConfig),
    });
  } catch (error) {
    console.error("Failed to find workspace:", error);
    return NextResponse.json({ error: "Failed to find workspace" }, { status: 500 });
  }
}
