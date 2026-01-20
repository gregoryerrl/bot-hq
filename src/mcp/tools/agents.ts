import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";

// Stub implementation - will be replaced when persistent manager is implemented
export function registerAgentTools(server: McpServer) {
  // agent_list - List all agent sessions
  server.tool(
    "agent_list",
    "List all agent sessions with their status, workspace, and task info",
    {},
    async () => {
      // TODO: Will be replaced with persistent manager status
      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify({
              message: "Agent system is being migrated to persistent manager architecture",
              sessions: [],
            }, null, 2),
          },
        ],
      };
    }
  );

  // agent_start - Start an agent on a task
  server.tool(
    "agent_start",
    "Start a Claude Code agent to work on a specific task",
    {
      taskId: z.number().describe("The task ID to start working on"),
    },
    async ({ taskId }) => {
      // TODO: Will send command to persistent manager
      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify({
              success: false,
              message: `Agent system is being migrated. Task ${taskId} cannot be started via this tool yet.`,
            }),
          },
        ],
      };
    }
  );

  // agent_stop - Stop a running agent
  server.tool(
    "agent_stop",
    "Stop an agent that is currently working on a task",
    {
      taskId: z.number().describe("The task ID to stop"),
    },
    async ({ taskId }) => {
      // TODO: Will send stop command to persistent manager
      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify({
              success: false,
              message: `Agent system is being migrated. Task ${taskId} cannot be stopped via this tool yet.`,
            }),
          },
        ],
      };
    }
  );
}
