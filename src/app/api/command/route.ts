import { NextRequest, NextResponse } from "next/server";
import { spawn } from "child_process";
import path from "path";
import { db, projects } from "@/lib/db";
import { eq } from "drizzle-orm";

export async function POST(request: NextRequest) {
  const body = await request.json();

  if (!body.input || typeof body.input !== "string") {
    return NextResponse.json({ error: "input is required" }, { status: 400 });
  }

  const context = body.context as
    | { projectId?: number; diagramId?: number; taskId?: number; label?: string }
    | undefined;

  let projectName: string | undefined;
  let repoPath: string | undefined;

  if (context?.projectId) {
    const project = await db.query.projects.findFirst({
      where: eq(projects.id, context.projectId),
    });
    if (project) {
      projectName = project.name;
      repoPath = project.repoPath ?? undefined;
    }
  }

  const contextParts: string[] = [];
  if (projectName && context?.projectId) {
    contextParts.push(
      `The user is currently viewing project "${projectName}" (ID: ${context.projectId}).`
    );
  }
  if (repoPath) {
    contextParts.push(`Repo path: ${repoPath}.`);
  }
  if (context?.diagramId) {
    contextParts.push(`They are looking at diagram ID ${context.diagramId}.`);
  }
  if (context?.label) {
    contextParts.push(`Current view: ${context.label}.`);
  }
  if (context?.taskId) {
    contextParts.push(`They are viewing task ID ${context.taskId}.`);
  }

  const contextInfo = contextParts.length > 0 ? " " + contextParts.join(" ") : "";

  const systemPrompt = `You are the bot-hq assistant. You help the user manage projects, tasks, and visualizer diagrams using the available MCP tools.${contextInfo}\n\nRespond concisely. If the user asks you to create, update, or delete something, use the appropriate MCP tool and confirm what you did. If they ask a question, answer it directly.`;

  const cwd = repoPath ? path.resolve(repoPath) : process.cwd();

  return new Promise<NextResponse>((resolve) => {
    const child = spawn("claude", ["--print", "-s", systemPrompt, "-p", body.input], {
      cwd,
      env: {
        ...process.env,
        CLAUDE_CODE_SESSION: undefined,
        CLAUDE_CODE_ENTRY_POINT: undefined,
      },
      timeout: 120000,
    });

    let stdout = "";
    let stderr = "";

    child.stdout.on("data", (data: Buffer) => {
      stdout += data.toString();
    });

    child.stderr.on("data", (data: Buffer) => {
      stderr += data.toString();
    });

    child.on("close", (code) => {
      if (code !== 0) {
        resolve(
          NextResponse.json(
            { error: stderr.trim() || `Process exited with code ${code}` },
            { status: 500 }
          )
        );
        return;
      }
      resolve(NextResponse.json({ response: stdout.trim() }));
    });

    child.on("error", (err) => {
      resolve(
        NextResponse.json(
          { error: err.message },
          { status: 500 }
        )
      );
    });
  });
}
