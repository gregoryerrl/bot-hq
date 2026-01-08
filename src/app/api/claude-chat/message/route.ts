import { NextRequest, NextResponse } from "next/server";
import { spawn } from "child_process";
import { getScopePath } from "@/lib/settings";
import path from "path";
import os from "os";
import fs from "fs/promises";

interface MessageRequest {
  sessionId: string;
  message: string;
  projectPath?: string;
}

export async function POST(request: NextRequest) {
  try {
    const { sessionId, message, projectPath }: MessageRequest =
      await request.json();

    if (!sessionId || !message) {
      return NextResponse.json(
        { error: "sessionId and message are required" },
        { status: 400 }
      );
    }

    const scopePath = await getScopePath();

    // Determine working directory
    let cwd = scopePath;
    if (projectPath) {
      // Validate project path is within scope
      const normalizedProjectPath = path.normalize(projectPath);
      if (!normalizedProjectPath.startsWith(scopePath)) {
        return NextResponse.json(
          { error: "Project path is outside of scope" },
          { status: 400 }
        );
      }
      cwd = normalizedProjectPath;

      // Verify directory exists
      try {
        await fs.access(cwd);
      } catch {
        return NextResponse.json(
          { error: "Project directory does not exist" },
          { status: 400 }
        );
      }
    }

    // Run Claude Code with --resume flag
    const response = await runClaudeCode(sessionId, message, cwd);

    return NextResponse.json({ response });
  } catch (error) {
    console.error("Failed to send message to Claude Code:", error);
    return NextResponse.json(
      { error: `Failed to send message: ${error}` },
      { status: 500 }
    );
  }
}

function runClaudeCode(
  sessionId: string,
  message: string,
  cwd: string
): Promise<string> {
  return new Promise((resolve, reject) => {
    const claudePath = process.env.CLAUDE_PATH || "claude";

    console.log("[Claude Chat Message] Spawning claude...");
    console.log("[Claude Chat Message] Path:", claudePath);
    console.log("[Claude Chat Message] Session:", sessionId);
    console.log("[Claude Chat Message] CWD:", cwd);

    const child = spawn(claudePath, ["--resume", sessionId, "-p", message, "--output-format", "text"], {
      cwd,
      env: {
        ...process.env,
        HOME: os.homedir(),
      },
      stdio: ["ignore", "pipe", "pipe"],
    });

    let stdout = "";
    let stderr = "";

    child.stdout.on("data", (data) => {
      stdout += data.toString();
    });

    child.stderr.on("data", (data) => {
      stderr += data.toString();
      console.log("[Claude Chat Message] stderr chunk:", data.toString());
    });

    child.on("error", (err) => {
      console.error("[Claude Chat Message] Spawn error:", err);
      reject(err);
    });

    child.on("close", (code) => {
      console.log("[Claude Chat Message] Process closed with code:", code);
      console.log("[Claude Chat Message] stdout length:", stdout.length);

      if (code === 0) {
        resolve(stdout.trim());
      } else {
        reject(new Error(stderr || `Process exited with code ${code}`));
      }
    });

    // Timeout after 5 minutes
    setTimeout(() => {
      child.kill("SIGTERM");
      reject(new Error("Claude Code timed out after 5 minutes"));
    }, 5 * 60 * 1000);
  });
}
