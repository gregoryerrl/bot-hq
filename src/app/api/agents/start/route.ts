import { NextRequest, NextResponse } from "next/server";
import { startAgentForTask } from "@/lib/agents/claude-code";

// Store active agents in memory (in production, use Redis or similar)
const activeAgents = new Map<number, ReturnType<typeof startAgentForTask>>();

export async function POST(request: NextRequest) {
  try {
    const { taskId } = await request.json();

    if (!taskId) {
      return NextResponse.json(
        { error: "taskId is required" },
        { status: 400 }
      );
    }

    // Check if agent already running for this task
    if (activeAgents.has(taskId)) {
      return NextResponse.json(
        { error: "Agent already running for this task" },
        { status: 400 }
      );
    }

    const agent = await startAgentForTask(taskId);
    if (!agent) {
      return NextResponse.json(
        { error: "Failed to start agent" },
        { status: 500 }
      );
    }

    activeAgents.set(taskId, Promise.resolve(agent));

    return NextResponse.json({ status: "started", taskId });
  } catch (error) {
    console.error("Failed to start agent:", error);
    return NextResponse.json(
      { error: "Failed to start agent" },
      { status: 500 }
    );
  }
}
