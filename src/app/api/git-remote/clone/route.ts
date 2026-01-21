import { NextRequest, NextResponse } from "next/server";
import { db, gitRemotes, workspaces, settings } from "@/lib/db";
import { eq } from "drizzle-orm";
import { exec } from "child_process";
import { promisify } from "util";
import path from "path";
import fs from "fs/promises";

const execAsync = promisify(exec);

function decryptCredentials(encrypted: string): { token: string } | null {
  try {
    return JSON.parse(Buffer.from(encrypted, "base64").toString("utf-8"));
  } catch {
    return null;
  }
}

function encryptCredentials(token: string): string {
  return Buffer.from(JSON.stringify({ token })).toString("base64");
}

export async function POST(request: NextRequest) {
  try {
    const { owner, repo, localPath } = await request.json();

    if (!owner || !repo) {
      return NextResponse.json(
        { error: "Missing required fields: owner, repo" },
        { status: 400 }
      );
    }

    // Find a GitHub remote with credentials
    const remote = await db.query.gitRemotes.findFirst({
      where: eq(gitRemotes.provider, "github"),
    });

    if (!remote?.credentials) {
      return NextResponse.json(
        { error: "No GitHub credentials configured" },
        { status: 400 }
      );
    }

    const creds = decryptCredentials(remote.credentials);
    if (!creds?.token) {
      return NextResponse.json(
        { error: "Invalid credentials" },
        { status: 400 }
      );
    }

    // Determine clone path
    let clonePath = localPath;
    if (!clonePath) {
      // Get scope path from settings
      const scopeSetting = await db.query.settings.findFirst({
        where: eq(settings.key, "scopePath"),
      });
      const scopePath = scopeSetting?.value || process.cwd();
      clonePath = path.join(scopePath, repo);
    }

    // Check if path exists
    try {
      await fs.access(clonePath);
      return NextResponse.json(
        { error: `Directory already exists: ${clonePath}` },
        { status: 400 }
      );
    } catch {
      // Path doesn't exist, good
    }

    // Clone with token
    const cloneUrl = `https://${creds.token}@github.com/${owner}/${repo}.git`;

    try {
      await execAsync(`git clone "${cloneUrl}" "${clonePath}"`, {
        timeout: 120000, // 2 minute timeout
      });
    } catch (error) {
      console.error("Clone failed:", error);
      return NextResponse.json(
        { error: "Failed to clone repository" },
        { status: 500 }
      );
    }

    // Check if workspace with same name exists
    const existing = await db.query.workspaces.findFirst({
      where: eq(workspaces.name, repo),
    });

    if (existing) {
      return NextResponse.json({
        workspaceName: repo,
        workspaceId: existing.id,
        message: "Repository cloned, workspace already exists",
        path: clonePath,
      });
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

    // Create git remote for this workspace
    await db.insert(gitRemotes).values({
      workspaceId: newWorkspace.id,
      provider: "github",
      name: `${owner}/${repo}`,
      url: "https://github.com",
      owner,
      repo,
      credentials: encryptCredentials(creds.token),
      createdAt: new Date(),
      updatedAt: new Date(),
    });

    return NextResponse.json({
      workspaceName: newWorkspace.name,
      workspaceId: newWorkspace.id,
      message: "Repository cloned and workspace created",
      path: clonePath,
    });
  } catch (error) {
    console.error("Clone failed:", error);
    return NextResponse.json(
      { error: "Failed to clone repository" },
      { status: 500 }
    );
  }
}
