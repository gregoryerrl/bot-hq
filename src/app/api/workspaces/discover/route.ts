import { NextRequest, NextResponse } from "next/server";
import { discoverWorkspaces, scanForCleanup } from "@/lib/workspace-discovery";
import { db, workspaces, logs } from "@/lib/db";
import { eq } from "drizzle-orm";
import { initializeWorkspaceContext } from "@/lib/bot-hq";
import { detectGitRemotes, createWorkspaceRemote } from "@/lib/git-remote-utils";
import { execSync } from "child_process";
import path from "path";

export async function GET() {
  try {
    const [workspaceSuggestions, cleanupSuggestions] = await Promise.all([
      discoverWorkspaces(),
      scanForCleanup(),
    ]);

    return NextResponse.json({
      workspaces: workspaceSuggestions,
      cleanup: cleanupSuggestions,
    });
  } catch (error) {
    console.error("Discovery scan failed:", error);
    return NextResponse.json(
      { error: "Discovery scan failed" },
      { status: 500 }
    );
  }
}

export async function POST(req: NextRequest) {
  try {
    const body = await req.json();
    const { action } = body;

    if (action === "add_workspace") {
      const { name, repoPath } = body;
      if (!name || !repoPath) {
        return NextResponse.json(
          { error: "name and repoPath are required" },
          { status: 400 }
        );
      }

      // Check if workspace already exists
      const existing = await db.query.workspaces.findFirst({
        where: eq(workspaces.name, name),
      });
      if (existing) {
        return NextResponse.json(
          { error: `Workspace '${name}' already exists` },
          { status: 409 }
        );
      }

      const [newWorkspace] = await db
        .insert(workspaces)
        .values({
          name,
          repoPath,
          createdAt: new Date(),
        })
        .returning();

      await initializeWorkspaceContext(name);

      // Auto-detect and register git remotes
      let autoDetectedRemotes = 0;
      try {
        const remotes = await detectGitRemotes(repoPath);
        for (const remote of remotes) {
          const created = await createWorkspaceRemote(newWorkspace.id, remote);
          if (created) autoDetectedRemotes++;
        }
      } catch {
        // Remote detection failure shouldn't block workspace creation
      }

      await db.insert(logs).values({
        workspaceId: newWorkspace.id,
        type: "agent",
        message: `Workspace created via discovery: ${name} -> ${repoPath}${autoDetectedRemotes > 0 ? ` (${autoDetectedRemotes} remote${autoDetectedRemotes > 1 ? "s" : ""} auto-detected)` : ""}`,
      });

      return NextResponse.json({
        success: true,
        workspaceId: newWorkspace.id,
        autoDetectedRemotes,
      });
    }

    if (action === "delete_folder") {
      const { path: folderPath } = body;
      if (!folderPath) {
        return NextResponse.json(
          { error: "path is required" },
          { status: 400 }
        );
      }

      // Safety: resolve path and ensure it's a real directory
      const resolved = path.resolve(folderPath);
      try {
        // Move to macOS Trash (safe, reversible)
        execSync(`osascript -e 'tell application "Finder" to delete POSIX file "${resolved}"'`);
      } catch (err) {
        console.error("Failed to move to trash:", err);
        return NextResponse.json(
          { error: "Failed to move folder to Trash" },
          { status: 500 }
        );
      }

      await db.insert(logs).values({
        type: "agent",
        message: `Folder moved to Trash via cleanup: ${resolved}`,
      });

      return NextResponse.json({ success: true });
    }

    return NextResponse.json(
      { error: `Unknown action: ${action}` },
      { status: 400 }
    );
  } catch (error) {
    console.error("Discovery action failed:", error);
    return NextResponse.json(
      { error: "Action failed" },
      { status: 500 }
    );
  }
}
