import { NextRequest, NextResponse } from "next/server";
import { execFile } from "child_process";
import { promisify } from "util";
import { getPromptRecordBySlug } from "@/lib/prompts";

const execFileAsync = promisify(execFile);

export async function POST(
  request: NextRequest,
  { params }: { params: Promise<{ slug: string }> }
) {
  try {
    const { slug } = await params;
    const body = await request.json();

    if (!body.instruction || typeof body.instruction !== "string") {
      return NextResponse.json(
        { error: "instruction is required" },
        { status: 400 }
      );
    }

    const prompt = await getPromptRecordBySlug(slug);
    if (!prompt) {
      return NextResponse.json({ error: "Prompt not found" }, { status: 404 });
    }

    const promptText = `You are a prompt engineering expert. Below is an agent prompt used in an AI orchestration system (bot-hq). The user wants you to improve it.

## Current Prompt
\`\`\`
${prompt.content}
\`\`\`

## User's Improvement Request
${body.instruction}

## Instructions
- Return ONLY the improved prompt text, no explanations or commentary
- Preserve the overall structure and format
- Keep all {{variable}} placeholders intact if present
- Make targeted improvements based on the user's request
- Do not add unnecessary verbosity`;

    // Strip Claude Code env vars so the child process doesn't detect nesting
    const cleanEnv = { ...process.env };
    delete cleanEnv.CLAUDE_CODE_ENTRYPOINT;
    delete cleanEnv.CLAUDE_CODE_MAX_OUTPUT_TOKENS;
    delete cleanEnv.CLAUDECODE;

    const { stdout } = await execFileAsync("claude", [
      "-p",
      "--print",
      "--model", "sonnet",
      promptText,
    ], { timeout: 60000, env: cleanEnv });

    return NextResponse.json({ suggestion: stdout.trim() });
  } catch (error) {
    console.error("Failed to improve prompt:", error);
    const message = error instanceof Error ? error.message : "Unknown error";
    return NextResponse.json(
      { error: `Failed to improve prompt: ${message}` },
      { status: 500 }
    );
  }
}
