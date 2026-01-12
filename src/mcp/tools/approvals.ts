import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { exec } from "child_process";
import { promisify } from "util";
import { db, approvals, tasks, workspaces, logs } from "../../lib/db/index.js";
import { eq, desc } from "drizzle-orm";
import { startAgentForTask } from "../../lib/agents/claude-code.js";

const execAsync = promisify(exec);

export function registerApprovalTools(server: McpServer) {
  // approval_list - List approvals
  server.tool(
    "approval_list",
    "List approvals awaiting review",
    {
      status: z
        .enum(["pending", "approved", "rejected"])
        .optional()
        .default("pending")
        .describe("Filter by approval status"),
    },
    async ({ status }) => {
      const approvalList = await db
        .select({
          id: approvals.id,
          taskId: approvals.taskId,
          workspaceId: approvals.workspaceId,
          branchName: approvals.branchName,
          baseBranch: approvals.baseBranch,
          status: approvals.status,
          createdAt: approvals.createdAt,
        })
        .from(approvals)
        .where(eq(approvals.status, status || "pending"))
        .orderBy(desc(approvals.createdAt))
        .limit(50);

      // Enrich with task and workspace info
      const enrichedApprovals = await Promise.all(
        approvalList.map(async (approval) => {
          const task = approval.taskId
            ? await db.query.tasks.findFirst({
                where: eq(tasks.id, approval.taskId),
              })
            : null;
          const workspace = await db.query.workspaces.findFirst({
            where: eq(workspaces.id, approval.workspaceId),
          });

          return {
            id: approval.id,
            taskId: approval.taskId,
            taskTitle: task?.title || "Unknown",
            workspaceName: workspace?.name || "Unknown",
            branchName: approval.branchName,
            baseBranch: approval.baseBranch,
            status: approval.status,
            createdAt: approval.createdAt?.toISOString(),
          };
        })
      );

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify(enrichedApprovals, null, 2),
          },
        ],
      };
    }
  );

  // approval_get - Get full approval details
  server.tool(
    "approval_get",
    "Get full details of an approval including diff and commits",
    {
      approvalId: z.number().describe("The approval ID to retrieve"),
    },
    async ({ approvalId }) => {
      const approval = await db.query.approvals.findFirst({
        where: eq(approvals.id, approvalId),
      });

      if (!approval) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({ error: `Approval ${approvalId} not found` }),
            },
          ],
        };
      }

      const task = approval.taskId
        ? await db.query.tasks.findFirst({
            where: eq(tasks.id, approval.taskId),
          })
        : null;
      const workspace = await db.query.workspaces.findFirst({
        where: eq(workspaces.id, approval.workspaceId),
      });

      // Parse stored JSON
      let commitMessages: string[] = [];
      let diffSummary = {};
      try {
        commitMessages = approval.commitMessages
          ? JSON.parse(approval.commitMessages)
          : [];
        diffSummary = approval.diffSummary
          ? JSON.parse(approval.diffSummary)
          : {};
      } catch {
        // Ignore parse errors
      }

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify(
              {
                id: approval.id,
                taskId: approval.taskId,
                taskTitle: task?.title || "Unknown",
                taskDescription: task?.description || "",
                workspaceName: workspace?.name || "Unknown",
                repoPath: workspace?.repoPath || "Unknown",
                branchName: approval.branchName,
                baseBranch: approval.baseBranch,
                status: approval.status,
                commitMessages,
                diffSummary,
                createdAt: approval.createdAt?.toISOString(),
              },
              null,
              2
            ),
          },
        ],
      };
    }
  );

  // approval_approve - Approve and create PR
  server.tool(
    "approval_approve",
    "Approve work - pushes branch to remote and creates PR on GitHub",
    {
      approvalId: z.number().describe("The approval ID to approve"),
      docRequest: z
        .string()
        .optional()
        .describe("Optional: request agent to write documentation after merge"),
    },
    async ({ approvalId, docRequest }) => {
      const approval = await db.query.approvals.findFirst({
        where: eq(approvals.id, approvalId),
      });

      if (!approval) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({
                success: false,
                message: `Approval ${approvalId} not found`,
              }),
            },
          ],
        };
      }

      if (approval.status !== "pending") {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({
                success: false,
                message: `Approval ${approvalId} is not pending (current: ${approval.status})`,
              }),
            },
          ],
        };
      }

      const workspace = await db.query.workspaces.findFirst({
        where: eq(workspaces.id, approval.workspaceId),
      });

      if (!workspace) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({
                success: false,
                message: "Workspace not found",
              }),
            },
          ],
        };
      }

      const repoPath = workspace.repoPath.replace("~", process.env.HOME || "");

      try {
        // Push branch to remote
        await execAsync(`git push -u origin ${approval.branchName}`, {
          cwd: repoPath,
        });

        // Get task for PR title
        const task = approval.taskId
          ? await db.query.tasks.findFirst({
              where: eq(tasks.id, approval.taskId),
            })
          : null;

        // Create PR using gh CLI
        const prTitle = task?.title || `Feature: ${approval.branchName}`;
        const prBody = `Automated PR from Bot-HQ\n\n${task?.description || ""}`;

        const { stdout: prOutput } = await execAsync(
          `gh pr create --title "${prTitle.replace(/"/g, '\\"')}" --body "${prBody.replace(/"/g, '\\"')}" --head ${approval.branchName} --base ${approval.baseBranch}`,
          { cwd: repoPath }
        );

        // Extract PR URL
        const prUrlMatch = prOutput.match(/(https:\/\/github\.com\/[^\s]+)/);
        const prUrl = prUrlMatch ? prUrlMatch[1] : prOutput.trim();

        // Update task state to done
        if (approval.taskId) {
          await db
            .update(tasks)
            .set({
              state: "done",
              updatedAt: new Date(),
            })
            .where(eq(tasks.id, approval.taskId));
        }

        // Update approval status
        await db
          .update(approvals)
          .set({ status: "approved" })
          .where(eq(approvals.id, approvalId));

        // Log
        await db.insert(logs).values({
          workspaceId: approval.workspaceId,
          taskId: approval.taskId,
          type: "approval",
          message: `PR created: ${prUrl}`,
        });

        // Handle doc request if provided
        if (docRequest && approval.taskId) {
          // Create follow-up task for documentation
          const [docTask] = await db
            .insert(tasks)
            .values({
              workspaceId: approval.workspaceId,
              title: `Documentation: ${task?.title || "Feature"}`,
              description: docRequest,
              state: "new",
              priority: 1,
            })
            .returning();

          await db.insert(logs).values({
            workspaceId: approval.workspaceId,
            taskId: docTask.id,
            type: "agent",
            message: `Documentation task created from approval`,
          });
        }

        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({
                success: true,
                prUrl,
                message: `PR created successfully`,
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
                message: `Failed to create PR: ${error}`,
              }),
            },
          ],
        };
      }
    }
  );

  // approval_request_changes - Send feedback and restart agent
  server.tool(
    "approval_request_changes",
    "Request changes - sends feedback and restarts agent to fix issues",
    {
      approvalId: z.number().describe("The approval ID"),
      feedback: z.string().describe("Specific instructions for what to fix"),
    },
    async ({ approvalId, feedback }) => {
      const approval = await db.query.approvals.findFirst({
        where: eq(approvals.id, approvalId),
      });

      if (!approval) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({
                success: false,
                message: `Approval ${approvalId} not found`,
              }),
            },
          ],
        };
      }

      if (approval.status !== "pending") {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({
                success: false,
                message: `Approval ${approvalId} is not pending`,
              }),
            },
          ],
        };
      }

      if (!approval.taskId) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({
                success: false,
                message: `No task associated with approval`,
              }),
            },
          ],
        };
      }

      // Store the feedback
      await db
        .update(approvals)
        .set({ userInstructions: feedback })
        .where(eq(approvals.id, approvalId));

      // Delete the approval (new one will be created when agent finishes)
      await db.delete(approvals).where(eq(approvals.id, approvalId));

      // Update task state back to in_progress
      await db
        .update(tasks)
        .set({ state: "in_progress", updatedAt: new Date() })
        .where(eq(tasks.id, approval.taskId));

      // Log the change request
      await db.insert(logs).values({
        workspaceId: approval.workspaceId,
        taskId: approval.taskId,
        type: "approval",
        message: `Changes requested: ${feedback.slice(0, 200)}...`,
      });

      // Restart the agent with the feedback
      const agent = await startAgentForTask(approval.taskId, feedback);

      if (!agent) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({
                success: false,
                message: "Failed to restart agent",
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
              message: `Agent restarted with feedback. Task ${approval.taskId} is back in progress.`,
            }),
          },
        ],
      };
    }
  );
}
