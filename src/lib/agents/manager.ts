import Anthropic from "@anthropic-ai/sdk";
import { db, workspaces, tasks, logs, agentSessions } from "@/lib/db";
import { eq, desc, sql } from "drizzle-orm";
import { MANAGER_SYSTEM_PROMPT, buildContextPrompt } from "./manager-prompts";

const anthropic = new Anthropic({
  apiKey: process.env.ANTHROPIC_API_KEY,
});

export interface ChatMessage {
  role: "user" | "assistant";
  content: string;
}

export async function getManagerContext() {
  const allWorkspaces = await db.select({
    name: workspaces.name,
    repoPath: workspaces.repoPath,
  }).from(workspaces);

  const taskCountsRaw = await db
    .select({
      state: tasks.state,
      count: sql<number>`count(*)`.as('count'),
    })
    .from(tasks)
    .groupBy(tasks.state);

  const taskCounts: Record<string, number> = {};
  for (const row of taskCountsRaw) {
    taskCounts[row.state] = row.count;
  }

  const recentLogs = await db
    .select()
    .from(logs)
    .orderBy(desc(logs.createdAt))
    .limit(10);

  const activeSessions = await db
    .select()
    .from(agentSessions)
    .where(eq(agentSessions.status, "running"));

  return {
    workspaces: allWorkspaces,
    taskCounts,
    recentLogs,
    activeSessions: activeSessions.length,
  };
}

export async function chatWithManager(
  messages: ChatMessage[],
  onChunk?: (chunk: string) => void
): Promise<string> {
  const context = await getManagerContext();
  const contextPrompt = buildContextPrompt(context);

  const response = await anthropic.messages.create({
    model: "claude-3-haiku-20240307",
    max_tokens: 1024,
    system: `${MANAGER_SYSTEM_PROMPT}\n\n${contextPrompt}`,
    messages: messages.map(m => ({
      role: m.role,
      content: m.content,
    })),
    stream: true,
  });

  let fullResponse = "";

  for await (const event of response) {
    if (event.type === "content_block_delta" && event.delta.type === "text_delta") {
      const text = event.delta.text;
      fullResponse += text;
      onChunk?.(text);
    }
  }

  return fullResponse;
}

export async function getQuickSummary(): Promise<string> {
  const context = await getManagerContext();

  const totalTasks = Object.values(context.taskCounts).reduce((a, b) => a + b, 0);
  const inProgress = (context.taskCounts["in_progress"] || 0) +
                     (context.taskCounts["analyzing"] || 0);
  const pendingReview = context.taskCounts["plan_ready"] || 0;
  const done = context.taskCounts["done"] || 0;

  return `${totalTasks} tasks | ${inProgress} in progress | ${pendingReview} pending review | ${done} done | ${context.activeSessions} agents active`;
}
