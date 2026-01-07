import { NextRequest, NextResponse } from "next/server";
import { exec } from "child_process";
import { promisify } from "util";
import fs from "fs/promises";
import path from "path";

const execAsync = promisify(exec);

export async function POST(request: NextRequest) {
  try {
    const body = await request.json();
    const { path: repoPath, remote } = body;

    if (!repoPath || !remote) {
      return NextResponse.json(
        { error: "path and remote are required" },
        { status: 400 }
      );
    }

    // Expand ~ to home directory
    const expandedPath = repoPath.startsWith("~")
      ? repoPath.replace("~", process.env.HOME || "")
      : repoPath;

    // Check if directory exists
    let dirExists = false;
    let isEmpty = true;

    try {
      const stats = await fs.stat(expandedPath);
      dirExists = stats.isDirectory();

      if (dirExists) {
        const files = await fs.readdir(expandedPath);
        isEmpty = files.length === 0;
      }
    } catch (error) {
      // Directory doesn't exist, which is fine - we'll clone into it
      dirExists = false;
    }

    // If directory exists and has content, skip cloning
    if (dirExists && !isEmpty) {
      console.log(`Directory ${expandedPath} exists with content, skipping clone`);
      return NextResponse.json({ success: true, skipped: true });
    }

    // Clone the repository
    console.log(`Cloning ${remote} into ${expandedPath}...`);

    try {
      const cloneCommand = dirExists && isEmpty
        ? `gh repo clone ${remote} ${expandedPath}`
        : `gh repo clone ${remote} ${expandedPath}`;

      const { stdout, stderr } = await execAsync(cloneCommand, {
        timeout: 120000, // 2 minute timeout
      });

      console.log(`Clone output: ${stdout}`);
      if (stderr) {
        console.error(`Clone stderr: ${stderr}`);
      }

      return NextResponse.json({ success: true, cloned: true });
    } catch (error: any) {
      console.error("Clone failed:", error);
      return NextResponse.json(
        { error: `Failed to clone repository: ${error.message}` },
        { status: 500 }
      );
    }
  } catch (error: any) {
    console.error("Failed to clone workspace:", error);
    return NextResponse.json(
      { error: error.message || "Failed to clone workspace" },
      { status: 500 }
    );
  }
}
