import { NextRequest, NextResponse } from "next/server";
import { spawn } from "child_process";
import { getPromptRecordBySlug } from "@/lib/prompts";

function runClaude(prompt: string, env: NodeJS.ProcessEnv): Promise<string> {
  return new Promise((resolve, reject) => {
    const proc = spawn("claude", ["-p", "--print", "--model", "sonnet"], {
      env,
      timeout: 60000,
    });

    let stdout = "";
    let stderr = "";

    proc.stdout.on("data", (d) => { stdout += d; });
    proc.stderr.on("data", (d) => { stderr += d; });

    proc.on("close", (code) => {
      if (code === 0) resolve(stdout);
      else reject(new Error(stderr || `Process exited with code ${code}`));
    });
    proc.on("error", reject);

    proc.stdin.write(prompt);
    proc.stdin.end();
  });
}

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

    const promptText = `You are a prompt engineering expert. Your ONLY job is to rewrite the prompt below based on the user's request.

CRITICAL RULES:
- Output ONLY the full rewritten prompt text. Nothing else.
- Do NOT explain what you changed. Do NOT add commentary, code snippets, file paths, or instructions.
- Do NOT suggest code changes. You are rewriting the PROMPT TEXT, not the application code.
- Preserve all {{variable}} mustache placeholders exactly as they appear.
- Preserve the overall markdown structure and format.
- Make targeted improvements based on the user's request.

## Current Prompt
\`\`\`
${prompt.content}
\`\`\`

## User's Improvement Request
${body.instruction}

Remember: Output the complete improved prompt and NOTHING else. No preamble, no explanation.`;

    // Strip Claude Code env vars so the child process doesn't detect nesting
    const cleanEnv = { ...process.env };
    delete cleanEnv.CLAUDE_CODE_ENTRYPOINT;
    delete cleanEnv.CLAUDE_CODE_MAX_OUTPUT_TOKENS;
    delete cleanEnv.CLAUDECODE;

    const result = await runClaude(promptText, cleanEnv);

    return NextResponse.json({ suggestion: result.trim() });
  } catch (error) {
    console.error("Failed to improve prompt:", error);
    const message = error instanceof Error ? error.message : "Unknown error";
    return NextResponse.json(
      { error: `Failed to improve prompt: ${message}` },
      { status: 500 }
    );
  }
}
