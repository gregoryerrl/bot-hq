import { NextRequest, NextResponse } from "next/server";
import fs from "fs/promises";
import path from "path";
import os from "os";

interface Message {
  role: "user" | "assistant";
  content: string;
  timestamp: string;
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

export async function GET(
  request: NextRequest,
  { params }: { params: Promise<{ sessionId: string }> }
) {
  try {
    const { sessionId } = await params;
    const claudeProjectsDir = path.join(os.homedir(), ".claude", "projects");

    // Search all project directories for this session
    const projectDirs = await fs.readdir(claudeProjectsDir);
    let sessionData: {
      messages: Message[];
      projectPath: string;
      projectName: string;
    } | null = null;

    for (const encodedPath of projectDirs) {
      const projectDir = path.join(claudeProjectsDir, encodedPath);

      try {
        const stat = await fs.stat(projectDir);
        if (!stat.isDirectory()) continue;
      } catch {
        continue;
      }

      const files = await fs.readdir(projectDir);
      const sessionFiles = files.filter((f) => f.endsWith(".jsonl"));

      for (const sessionFile of sessionFiles) {
        try {
          const filePath = path.join(projectDir, sessionFile);
          const content = await fs.readFile(filePath, "utf-8");
          const lines = content.trim().split("\n").filter(Boolean);

          let foundSession = false;
          let projectPath = "";
          const messages: Message[] = [];

          for (const line of lines) {
            try {
              const entry: SessionEntry = JSON.parse(line);

              // Check if this entry belongs to the session we're looking for
              if (entry.sessionId === sessionId) {
                foundSession = true;

                // Get project path from cwd field
                if (entry.cwd && !projectPath) {
                  projectPath = entry.cwd;
                }
              }

              // Only process messages if we've found the session
              if (foundSession) {
                if (entry.type === "user" && entry.message?.role === "user") {
                  const msgContent = entry.message.content;
                  let text = "";

                  if (typeof msgContent === "string") {
                    text = msgContent;
                  } else if (Array.isArray(msgContent)) {
                    const textPart = msgContent.find((p) => p.type === "text");
                    if (textPart?.text) {
                      text = textPart.text;
                    }
                  }

                  if (text) {
                    messages.push({
                      role: "user",
                      content: text,
                      timestamp: entry.timestamp || "",
                    });
                  }
                }

                if (entry.type === "assistant" && entry.message) {
                  const msgContent = entry.message.content;
                  let text = "";

                  if (typeof msgContent === "string") {
                    text = msgContent;
                  } else if (Array.isArray(msgContent)) {
                    // Combine all text parts
                    text = msgContent
                      .filter((p) => p.type === "text" && p.text)
                      .map((p) => p.text)
                      .join("\n");
                  }

                  if (text) {
                    messages.push({
                      role: "assistant",
                      content: text,
                      timestamp: entry.timestamp || "",
                    });
                  }
                }
              }
            } catch {
              // Skip malformed lines
            }
          }

          if (foundSession && messages.length > 0) {
            sessionData = {
              messages,
              projectPath: projectPath || "Unknown",
              projectName: projectPath ? path.basename(projectPath) : "Unknown",
            };
            break;
          }
        } catch {
          // Skip files that can't be read
        }
      }

      if (sessionData) break;
    }

    if (!sessionData) {
      return NextResponse.json({ error: "Session not found" }, { status: 404 });
    }

    return NextResponse.json(sessionData);
  } catch (error) {
    console.error("Failed to fetch session:", error);
    return NextResponse.json(
      { error: "Failed to fetch session" },
      { status: 500 }
    );
  }
}
