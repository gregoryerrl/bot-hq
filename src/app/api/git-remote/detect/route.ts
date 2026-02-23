import { NextRequest, NextResponse } from "next/server";
import { db, workspaces } from "@/lib/db";
import { eq } from "drizzle-orm";
import { detectGitRemotes, createWorkspaceRemote } from "@/lib/git-remote-utils";

export async function POST(req: NextRequest) {
  try {
    const body = await req.json().catch(() => ({}));
    const { workspaceId } = body as { workspaceId?: number };

    let targetWorkspaces: { id: number; name: string; repoPath: string }[];

    if (workspaceId) {
      const ws = await db.query.workspaces.findFirst({
        where: eq(workspaces.id, workspaceId),
      });
      if (!ws) {
        return NextResponse.json(
          { error: `Workspace ${workspaceId} not found` },
          { status: 404 }
        );
      }
      targetWorkspaces = [ws];
    } else {
      targetWorkspaces = await db.select().from(workspaces);
    }

    let totalDetected = 0;
    const results: { workspaceId: number; name: string; remotesAdded: number }[] = [];

    for (const ws of targetWorkspaces) {
      try {
        const remotes = await detectGitRemotes(ws.repoPath);
        let added = 0;
        for (const remote of remotes) {
          const created = await createWorkspaceRemote(ws.id, remote);
          if (created) added++;
        }
        if (added > 0) {
          results.push({ workspaceId: ws.id, name: ws.name, remotesAdded: added });
          totalDetected += added;
        }
      } catch {
        // Skip workspaces where git remote detection fails
      }
    }

    return NextResponse.json({
      success: true,
      totalDetected,
      workspacesScanned: targetWorkspaces.length,
      results,
    });
  } catch (error) {
    console.error("Remote detection failed:", error);
    return NextResponse.json(
      { error: "Remote detection failed" },
      { status: 500 }
    );
  }
}
