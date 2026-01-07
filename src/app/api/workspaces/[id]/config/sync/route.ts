import { NextRequest, NextResponse } from "next/server";
import { db, workspaces } from "@/lib/db";
import { eq } from "drizzle-orm";
import { parseAgentConfig } from "@/lib/agents/config-types";
import fs from "fs/promises";
import path from "path";

export async function POST(
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

    const repoPath = workspace[0].repoPath.replace("~", process.env.HOME || "");
    const claudeDir = path.join(repoPath, ".claude");
    const settingsPath = path.join(claudeDir, "settings.json");

    const config = parseAgentConfig(workspace[0].agentConfig);

    // Get Bot-HQ project root for hook path
    const botHqRoot = process.cwd();
    const hookPath = path.join(botHqRoot, ".claude", "hooks", "approval-gate.js");

    // Create .claude directory
    await fs.mkdir(claudeDir, { recursive: true });

    // Build settings.json
    const claudeSettings = {
      hooks: {
        PreToolUse: [{
          matcher: "Bash",
          hooks: [{
            type: "command",
            command: `node "${hookPath}"`
          }]
        }]
      }
    };

    await fs.writeFile(settingsPath, JSON.stringify(claudeSettings, null, 2), "utf-8");

    return NextResponse.json({ success: true, path: settingsPath });
  } catch (error) {
    console.error("Failed to sync config:", error);
    return NextResponse.json({ error: "Failed to sync config" }, { status: 500 });
  }
}
