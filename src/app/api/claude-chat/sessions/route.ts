import { NextResponse } from "next/server";
import { getScopePath } from "@/lib/settings";
import fs from "fs/promises";
import path from "path";
import os from "os";

interface SessionInfo {
  sessionId: string;
  projectPath: string;
  projectName: string;
  firstMessage: string;
  lastActivityAt: string;
  messageCount: number;
}

interface SessionEntry {
  type: string;
  sessionId?: string;
  message?: {
    role: string;
    content?: string | Array<{ type: string; text?: string }>;
  };
  timestamp?: string;
  cwd?: string;
}

export async function GET() {
  try {
    const scopePath = await getScopePath();
    const claudeProjectsDir = path.join(os.homedir(), ".claude", "projects");

    // Check if Claude projects directory exists
    try {
      await fs.access(claudeProjectsDir);
    } catch {
      return NextResponse.json({ sessions: [], scopePath });
    }

    // Read all project directories
    const projectDirs = await fs.readdir(claudeProjectsDir);
    const sessionMap = new Map<string, SessionInfo>();

    for (const encodedPath of projectDirs) {
      const projectDir = path.join(claudeProjectsDir, encodedPath);

      try {
        const stat = await fs.stat(projectDir);
        if (!stat.isDirectory()) continue;
      } catch {
        continue;
      }

      // Read session files in this project directory
      const files = await fs.readdir(projectDir);
      const sessionFiles = files.filter((f) => f.endsWith(".jsonl"));

      for (const sessionFile of sessionFiles) {
        try {
          const filePath = path.join(projectDir, sessionFile);
          const content = await fs.readFile(filePath, "utf-8");
          const lines = content.trim().split("\n").filter(Boolean);

          if (lines.length === 0) continue;

          let sessionId = "";
          let firstUserMessage = "";
          let lastTimestamp = "";
          let messageCount = 0;
          let projectPath = "";

          for (const line of lines) {
            try {
              const entry: SessionEntry = JSON.parse(line);

              // Get session ID from any entry that has it
              if (entry.sessionId && !sessionId) {
                sessionId = entry.sessionId;
              }

              // Get cwd (actual project path) from user entries
              if (entry.cwd && !projectPath) {
                projectPath = entry.cwd;
              }

              if (entry.type === "user" && entry.message?.role === "user") {
                messageCount++;
                if (entry.timestamp) {
                  lastTimestamp = entry.timestamp;
                }

                // Get first user message for preview
                if (!firstUserMessage) {
                  const content = entry.message.content;
                  if (typeof content === "string") {
                    firstUserMessage = content;
                  } else if (Array.isArray(content)) {
                    const textPart = content.find((p) => p.type === "text");
                    if (textPart?.text) {
                      firstUserMessage = textPart.text;
                    }
                  }
                }
              }

              if (entry.type === "assistant") {
                messageCount++;
                if (entry.timestamp) {
                  lastTimestamp = entry.timestamp;
                }
              }
            } catch {
              // Skip malformed lines
            }
          }

          // Only include sessions within the scope path
          if (!projectPath.startsWith(scopePath)) {
            continue;
          }

          if (sessionId && firstUserMessage) {
            // Use Map to dedupe by sessionId, keeping the one with latest activity
            const existing = sessionMap.get(sessionId);
            if (!existing || new Date(lastTimestamp) > new Date(existing.lastActivityAt)) {
              sessionMap.set(sessionId, {
                sessionId,
                projectPath,
                projectName: path.basename(projectPath),
                firstMessage:
                  firstUserMessage.length > 100
                    ? firstUserMessage.substring(0, 100) + "..."
                    : firstUserMessage,
                lastActivityAt: lastTimestamp,
                messageCount,
              });
            }
          }
        } catch {
          // Skip files that can't be read
        }
      }
    }

    // Convert Map to array and sort by last activity (most recent first)
    const sessions = Array.from(sessionMap.values()).sort(
      (a, b) =>
        new Date(b.lastActivityAt).getTime() -
        new Date(a.lastActivityAt).getTime()
    );

    return NextResponse.json({ sessions, scopePath });
  } catch (error) {
    console.error("Failed to fetch Claude Code sessions:", error);
    return NextResponse.json(
      { error: "Failed to fetch sessions" },
      { status: 500 }
    );
  }
}
