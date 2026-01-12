import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { db, agentSessions, tasks, workspaces } from "../../lib/db/index.js";
import { eq, desc } from "drizzle-orm";
import { startAgentForTask, ClaudeCodeAgent } from "../../lib/agents/claude-code.js";

// Track running agents
const runningAgents = new Map<number, ClaudeCodeAgent>();

export function registerAgentTools(server: McpServer) {
  // agent_list - List all agent sessions
  server.tool(
    "agent_list",
    "List all agent sessions with their status, workspace, and task info",
    {},
    async () => {
      const sessions = await db
        .select({
          id: agentSessions.id,
          status: agentSessions.status,
          pid: agentSessions.pid,
          workspaceId: agentSessions.workspaceId,
          taskId: agentSessions.taskId,
          startedAt: agentSessions.startedAt,
          lastActivityAt: agentSessions.lastActivityAt,
        })
        .from(agentSessions)
        .orderBy(desc(agentSessions.startedAt))
        .limit(50);

      // Enrich with workspace and task names
      const enrichedSessions = await Promise.all(
        sessions.map(async (session) => {
          const workspace = await db.query.workspaces.findFirst({
            where: eq(workspaces.id, session.workspaceId),
          });
          const task = session.taskId
            ? await db.query.tasks.findFirst({
                where: eq(tasks.id, session.taskId),
              })
            : null;

          return {
            id: session.id,
            status: session.status,
            pid: session.pid,
            workspaceName: workspace?.name || "Unknown",
            taskId: session.taskId,
            taskTitle: task?.title || "No task",
            startedAt: session.startedAt?.toISOString(),
            lastActivityAt: session.lastActivityAt?.toISOString(),
          };
        })
      );

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify(enrichedSessions, null, 2),
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
      // Check if agent already running for this task
      const existingSession = await db.query.agentSessions.findFirst({
        where: eq(agentSessions.taskId, taskId),
      });

      if (existingSession?.status === "running") {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({
                success: false,
                message: `Agent already running for task ${taskId}`,
              }),
            },
          ],
        };
      }

      // Start the agent
      const agent = await startAgentForTask(taskId);

      if (!agent) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({
                success: false,
                message: `Failed to start agent - task ${taskId} not found or no workspace`,
              }),
            },
          ],
        };
      }

      runningAgents.set(taskId, agent);

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify({
              success: true,
              message: `Agent started for task ${taskId}`,
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
      // Try in-memory agent first
      const agent = runningAgents.get(taskId);
      if (agent) {
        await agent.stop();
        runningAgents.delete(taskId);
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({
                success: true,
                message: `Agent stopped for task ${taskId}`,
              }),
            },
          ],
        };
      }

      // Try to find by PID in database
      const session = await db.query.agentSessions.findFirst({
        where: eq(agentSessions.taskId, taskId),
      });

      if (session?.pid && session.status === "running") {
        try {
          process.kill(session.pid, "SIGTERM");
          await db
            .update(agentSessions)
            .set({ status: "stopped" })
            .where(eq(agentSessions.id, session.id));

          return {
            content: [
              {
                type: "text" as const,
                text: JSON.stringify({
                  success: true,
                  message: `Agent stopped (PID: ${session.pid})`,
                }),
              },
            ],
          };
        } catch (error) {
          return {
            content: [
              {
                type: "text" as const,
                text: JSON.stringify({
                  success: false,
                  message: `Failed to stop agent: ${error}`,
                }),
              },
            ],
          };
        }
      }

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify({
              success: false,
              message: `No running agent found for task ${taskId}`,
            }),
          },
        ],
      };
    }
  );
}
