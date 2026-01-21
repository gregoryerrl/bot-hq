import { NextResponse } from "next/server";
import { db, plugins, workspaces, pluginWorkspaceData } from "@/lib/db";
import { initializePlugins } from "@/lib/plugins";
import { getScopePath } from "@/lib/settings";
import { eq, and } from "drizzle-orm";
import { exec } from "child_process";
import { promisify } from "util";
import path from "path";
import fs from "fs";

const execAsync = promisify(exec);

export async function POST(request: Request) {
  try {
    const { owner, repo, localPath } = await request.json();

    if (!owner || !repo) {
      return NextResponse.json(
        { error: "Owner and repo are required" },
        { status: 400 }
      );
    }

    await initializePlugins();

    // Get GitHub plugin
    const plugin = await db.query.plugins.findFirst({
      where: eq(plugins.name, "github"),
    });

    if (!plugin) {
      return NextResponse.json(
        { error: "GitHub plugin not installed" },
        { status: 400 }
      );
    }

    const credentials = plugin.credentials ? JSON.parse(plugin.credentials) : {};
    const token = credentials.GITHUB_TOKEN;

    if (!token) {
      return NextResponse.json(
        { error: "GitHub token not configured" },
        { status: 400 }
      );
    }

    // Determine the local path
    let clonePath = localPath;
    if (!clonePath) {
      const scopePath = await getScopePath();
      clonePath = path.join(scopePath, repo);
    }

    // Check if path already exists
    if (fs.existsSync(clonePath)) {
      return NextResponse.json(
        { error: `Directory already exists: ${clonePath}` },
        { status: 400 }
      );
    }

    // Ensure parent directory exists
    const parentDir = path.dirname(clonePath);
    if (!fs.existsSync(parentDir)) {
      fs.mkdirSync(parentDir, { recursive: true });
    }

    // Clone the repository using the token for authentication
    const cloneUrl = `https://${token}@github.com/${owner}/${repo}.git`;

    try {
      await execAsync(`git clone "${cloneUrl}" "${clonePath}"`, {
        timeout: 300000, // 5 minute timeout
      });
    } catch (cloneError) {
      // Clean up partial clone if it exists
      if (fs.existsSync(clonePath)) {
        fs.rmSync(clonePath, { recursive: true, force: true });
      }
      throw new Error(
        `Failed to clone repository: ${
          cloneError instanceof Error ? cloneError.message : "Unknown error"
        }`
      );
    }

    // Check if workspace already exists
    const existingWorkspace = await db.query.workspaces.findFirst({
      where: eq(workspaces.repoPath, clonePath),
    });

    if (existingWorkspace) {
      return NextResponse.json(
        { error: `Workspace already exists for this path` },
        { status: 400 }
      );
    }

    // Create workspace
    const [newWorkspace] = await db
      .insert(workspaces)
      .values({
        name: repo,
        repoPath: clonePath,
        createdAt: new Date(),
      })
      .returning();

    // Save GitHub data for this workspace
    await db.insert(pluginWorkspaceData).values({
      pluginId: plugin.id,
      workspaceId: newWorkspace.id,
      data: JSON.stringify({ owner, repo }),
      createdAt: new Date(),
      updatedAt: new Date(),
    });

    return NextResponse.json({
      success: true,
      workspaceId: newWorkspace.id,
      workspaceName: newWorkspace.name,
      path: clonePath,
    });
  } catch (error) {
    console.error("Failed to clone repository:", error);
    return NextResponse.json(
      { error: error instanceof Error ? error.message : "Failed to clone repository" },
      { status: 500 }
    );
  }
}
