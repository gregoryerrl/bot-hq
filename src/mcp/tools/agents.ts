import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";

// MCP server runs as separate process, so we need to use HTTP API to communicate with the Next.js server
const BOT_HQ_URL = process.env.BOT_HQ_URL || "http://localhost:7890";

async function sendCommandToManager(command: string): Promise<boolean> {
  try {
    const response = await fetch(`${BOT_HQ_URL}/api/manager/command`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ command }),
    });
    return response.ok;
  } catch (error) {
    console.error("Failed to send command to manager:", error);
    return false;
  }
}

async function getManagerStatus(): Promise<{ running: boolean; sessionId: string | null }> {
  try {
    const response = await fetch(`${BOT_HQ_URL}/api/terminal/manager`, {
      headers: { "Content-Type": "application/json" },
    });
    if (response.ok) {
      const data = await response.json();
      return { running: data.exists, sessionId: data.sessionId };
    }
    return { running: false, sessionId: null };
  } catch {
    return { running: false, sessionId: null };
  }
}

export function registerAgentTools(server: McpServer) {
  // agent_list - List manager status
  server.tool(
    "agent_list",
    "List all agent sessions with their status, workspace, and task info",
    {},
    async () => {
      const status = await getManagerStatus();
      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify({
              managerStatus: status.running ? "running" : "stopped",
              sessionId: status.sessionId,
              note: "Bot-hq uses a persistent manager terminal session that orchestrates subagents. Connect to the manager session via /api/terminal/manager",
            }, null, 2),
          },
        ],
      };
    }
  );

  // agent_start - Send start command to manager
  server.tool(
    "agent_start",
    "Start a Claude Code agent to work on a specific task",
    {
      taskId: z.number().describe("The task ID to start working on"),
    },
    async ({ taskId }) => {
      const status = await getManagerStatus();
      if (!status.running) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({
                success: false,
                error: "Manager is not running",
              }),
            },
          ],
        };
      }

      const success = await sendCommandToManager(
        `TASK ${taskId}`
      );

      if (!success) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({
                success: false,
                error: "Failed to send command to manager",
              }),
            },
          ],
        };
      }

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify({
              success: true,
              message: `Command sent to manager to start task ${taskId}`,
            }),
          },
        ],
      };
    }
  );

  // agent_stop - Send stop command to manager
  server.tool(
    "agent_stop",
    "Stop an agent that is currently working on a task",
    {
      taskId: z.number().describe("The task ID to stop"),
    },
    async ({ taskId }) => {
      const status = await getManagerStatus();
      if (!status.running) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({
                success: false,
                error: "Manager is not running",
              }),
            },
          ],
        };
      }

      const success = await sendCommandToManager(
        `Stop working on task ${taskId} if currently in progress.`
      );

      if (!success) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({
                success: false,
                error: "Failed to send stop command to manager",
              }),
            },
          ],
        };
      }

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify({
              success: true,
              message: `Command sent to manager to stop task ${taskId}`,
            }),
          },
        ],
      };
    }
  );
}
