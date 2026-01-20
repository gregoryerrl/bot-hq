import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { getManagerStatus, sendManagerCommand } from "../../lib/manager/persistent-manager.js";

export function registerAgentTools(server: McpServer) {
  // agent_list - List manager status
  server.tool(
    "agent_list",
    "List all agent sessions with their status, workspace, and task info",
    {},
    async () => {
      const status = getManagerStatus();
      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify({
              managerStatus: status.running ? "running" : "stopped",
              managerPid: status.pid,
              note: "Bot-hq now uses a single persistent manager that orchestrates subagents via the Task tool",
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
      const status = getManagerStatus();
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

      sendManagerCommand(
        `Start working on task ${taskId}. Use the task_get tool to fetch the task details, then spawn a subagent to work on it.`
      );

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
      const status = getManagerStatus();
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

      sendManagerCommand(`Stop working on task ${taskId} if currently in progress.`);

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
