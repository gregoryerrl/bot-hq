import { NextRequest, NextResponse } from "next/server";
import { spawn } from "child_process";
import { getScopePath } from "@/lib/settings";
import path from "path";
import os from "os";
import fs from "fs/promises";

interface NewSessionRequest {
  message: string;
  projectPath?: string;
}

export async function POST(request: NextRequest) {
  try {
    const { message, projectPath }: NewSessionRequest = await request.json();

    if (!message) {
      return NextResponse.json(
        { error: "message is required" },
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

    // Run Claude Code to start a new session
    const result = await runClaudeCode(message, cwd);

    return NextResponse.json(result);
  } catch (error) {
    console.error("Failed to start new Claude Code session:", error);
    return NextResponse.json(
      { error: `Failed to start session: ${error}` },
      { status: 500 }
    );
  }
}

function runClaudeCode(
  message: string,
  cwd: string
): Promise<{ response: string; sessionId: string | null }> {
  return new Promise((resolve, reject) => {
    const claudePath = process.env.CLAUDE_PATH || "claude";

    console.log("[Claude Chat] Spawning claude...");
    console.log("[Claude Chat] Path:", claudePath);
    console.log("[Claude Chat] CWD:", cwd);

    const child = spawn(claudePath, ["-p", message, "--output-format", "json"], {
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
      console.log("[Claude Chat] stderr chunk:", data.toString());
    });

    child.on("error", (err) => {
      console.error("[Claude Chat] Spawn error:", err);
      reject(err);
    });

    child.on("close", (code) => {
      console.log("[Claude Chat] Process closed with code:", code);
      console.log("[Claude Chat] stdout length:", stdout.length);

      if (code === 0) {
        try {
          const parsed = JSON.parse(stdout);
          resolve({
            response: parsed.result || "",
            sessionId: parsed.session_id || null,
          });
        } catch {
          resolve({ response: stdout.trim(), sessionId: null });
        }
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
