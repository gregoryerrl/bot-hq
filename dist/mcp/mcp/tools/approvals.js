"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.registerApprovalTools = registerApprovalTools;
const zod_1 = require("zod");
const child_process_1 = require("child_process");
const util_1 = require("util");
const index_js_1 = require("../../lib/db/index.js");
const drizzle_orm_1 = require("drizzle-orm");
const claude_code_js_1 = require("../../lib/agents/claude-code.js");
const execAsync = (0, util_1.promisify)(child_process_1.exec);
function registerApprovalTools(server) {
    // approval_list - List approvals
    server.tool("approval_list", "List approvals awaiting review", {
        status: zod_1.z
            .enum(["pending", "approved", "rejected"])
            .optional()
            .default("pending")
            .describe("Filter by approval status"),
    }, async ({ status }) => {
        const approvalList = await index_js_1.db
            .select({
            id: index_js_1.approvals.id,
            taskId: index_js_1.approvals.taskId,
            workspaceId: index_js_1.approvals.workspaceId,
            branchName: index_js_1.approvals.branchName,
            baseBranch: index_js_1.approvals.baseBranch,
            status: index_js_1.approvals.status,
            createdAt: index_js_1.approvals.createdAt,
        })
            .from(index_js_1.approvals)
            .where((0, drizzle_orm_1.eq)(index_js_1.approvals.status, status || "pending"))
            .orderBy((0, drizzle_orm_1.desc)(index_js_1.approvals.createdAt))
            .limit(50);
        // Enrich with task and workspace info
        const enrichedApprovals = await Promise.all(approvalList.map(async (approval) => {
            const task = approval.taskId
                ? await index_js_1.db.query.tasks.findFirst({
                    where: (0, drizzle_orm_1.eq)(index_js_1.tasks.id, approval.taskId),
                })
                : null;
            const workspace = await index_js_1.db.query.workspaces.findFirst({
                where: (0, drizzle_orm_1.eq)(index_js_1.workspaces.id, approval.workspaceId),
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
        }));
        return {
            content: [
                {
                    type: "text",
                    text: JSON.stringify(enrichedApprovals, null, 2),
                },
            ],
        };
    });
    // approval_get - Get full approval details
    server.tool("approval_get", "Get full details of an approval including diff and commits", {
        approvalId: zod_1.z.number().describe("The approval ID to retrieve"),
    }, async ({ approvalId }) => {
        const approval = await index_js_1.db.query.approvals.findFirst({
            where: (0, drizzle_orm_1.eq)(index_js_1.approvals.id, approvalId),
        });
        if (!approval) {
            return {
                content: [
                    {
                        type: "text",
                        text: JSON.stringify({ error: `Approval ${approvalId} not found` }),
                    },
                ],
            };
        }
        const task = approval.taskId
            ? await index_js_1.db.query.tasks.findFirst({
                where: (0, drizzle_orm_1.eq)(index_js_1.tasks.id, approval.taskId),
            })
            : null;
        const workspace = await index_js_1.db.query.workspaces.findFirst({
            where: (0, drizzle_orm_1.eq)(index_js_1.workspaces.id, approval.workspaceId),
        });
        // Parse stored JSON
        let commitMessages = [];
        let diffSummary = {};
        try {
            commitMessages = approval.commitMessages
                ? JSON.parse(approval.commitMessages)
                : [];
            diffSummary = approval.diffSummary
                ? JSON.parse(approval.diffSummary)
                : {};
        }
        catch {
            // Ignore parse errors
        }
        return {
            content: [
                {
                    type: "text",
                    text: JSON.stringify({
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
                    }, null, 2),
                },
            ],
        };
    });
    // approval_approve - Approve and create PR
    server.tool("approval_approve", "Approve work - pushes branch to remote and creates PR on GitHub", {
        approvalId: zod_1.z.number().describe("The approval ID to approve"),
        docRequest: zod_1.z
            .string()
            .optional()
            .describe("Optional: request agent to write documentation after merge"),
    }, async ({ approvalId, docRequest }) => {
        const approval = await index_js_1.db.query.approvals.findFirst({
            where: (0, drizzle_orm_1.eq)(index_js_1.approvals.id, approvalId),
        });
        if (!approval) {
            return {
                content: [
                    {
                        type: "text",
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
                        type: "text",
                        text: JSON.stringify({
                            success: false,
                            message: `Approval ${approvalId} is not pending (current: ${approval.status})`,
                        }),
                    },
                ],
            };
        }
        const workspace = await index_js_1.db.query.workspaces.findFirst({
            where: (0, drizzle_orm_1.eq)(index_js_1.workspaces.id, approval.workspaceId),
        });
        if (!workspace) {
            return {
                content: [
                    {
                        type: "text",
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
                ? await index_js_1.db.query.tasks.findFirst({
                    where: (0, drizzle_orm_1.eq)(index_js_1.tasks.id, approval.taskId),
                })
                : null;
            // Create PR using gh CLI
            const prTitle = task?.title || `Feature: ${approval.branchName}`;
            const prBody = `Automated PR from Bot-HQ\n\n${task?.description || ""}`;
            const { stdout: prOutput } = await execAsync(`gh pr create --title "${prTitle.replace(/"/g, '\\"')}" --body "${prBody.replace(/"/g, '\\"')}" --head ${approval.branchName} --base ${approval.baseBranch}`, { cwd: repoPath });
            // Extract PR URL
            const prUrlMatch = prOutput.match(/(https:\/\/github\.com\/[^\s]+)/);
            const prUrl = prUrlMatch ? prUrlMatch[1] : prOutput.trim();
            // Update task with PR URL
            if (approval.taskId) {
                await index_js_1.db
                    .update(index_js_1.tasks)
                    .set({
                    state: "pr_created",
                    prUrl,
                    updatedAt: new Date(),
                })
                    .where((0, drizzle_orm_1.eq)(index_js_1.tasks.id, approval.taskId));
            }
            // Update approval status
            await index_js_1.db
                .update(index_js_1.approvals)
                .set({ status: "approved" })
                .where((0, drizzle_orm_1.eq)(index_js_1.approvals.id, approvalId));
            // Log
            await index_js_1.db.insert(index_js_1.logs).values({
                workspaceId: approval.workspaceId,
                taskId: approval.taskId,
                type: "approval",
                message: `PR created: ${prUrl}`,
            });
            // Handle doc request if provided
            if (docRequest && approval.taskId) {
                // Create follow-up task for documentation
                const [docTask] = await index_js_1.db
                    .insert(index_js_1.tasks)
                    .values({
                    workspaceId: approval.workspaceId,
                    title: `Documentation: ${task?.title || "Feature"}`,
                    description: docRequest,
                    state: "new",
                    priority: 1,
                })
                    .returning();
                await index_js_1.db.insert(index_js_1.logs).values({
                    workspaceId: approval.workspaceId,
                    taskId: docTask.id,
                    type: "agent",
                    message: `Documentation task created from approval`,
                });
            }
            return {
                content: [
                    {
                        type: "text",
                        text: JSON.stringify({
                            success: true,
                            prUrl,
                            message: `PR created successfully`,
                        }),
                    },
                ],
            };
        }
        catch (error) {
            return {
                content: [
                    {
                        type: "text",
                        text: JSON.stringify({
                            success: false,
                            message: `Failed to create PR: ${error}`,
                        }),
                    },
                ],
            };
        }
    });
    // approval_request_changes - Send feedback and restart agent
    server.tool("approval_request_changes", "Request changes - sends feedback and restarts agent to fix issues", {
        approvalId: zod_1.z.number().describe("The approval ID"),
        feedback: zod_1.z.string().describe("Specific instructions for what to fix"),
    }, async ({ approvalId, feedback }) => {
        const approval = await index_js_1.db.query.approvals.findFirst({
            where: (0, drizzle_orm_1.eq)(index_js_1.approvals.id, approvalId),
        });
        if (!approval) {
            return {
                content: [
                    {
                        type: "text",
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
                        type: "text",
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
                        type: "text",
                        text: JSON.stringify({
                            success: false,
                            message: `No task associated with approval`,
                        }),
                    },
                ],
            };
        }
        // Store the feedback
        await index_js_1.db
            .update(index_js_1.approvals)
            .set({ userInstructions: feedback })
            .where((0, drizzle_orm_1.eq)(index_js_1.approvals.id, approvalId));
        // Delete the approval (new one will be created when agent finishes)
        await index_js_1.db.delete(index_js_1.approvals).where((0, drizzle_orm_1.eq)(index_js_1.approvals.id, approvalId));
        // Update task state back to in_progress
        await index_js_1.db
            .update(index_js_1.tasks)
            .set({ state: "in_progress", updatedAt: new Date() })
            .where((0, drizzle_orm_1.eq)(index_js_1.tasks.id, approval.taskId));
        // Log the change request
        await index_js_1.db.insert(index_js_1.logs).values({
            workspaceId: approval.workspaceId,
            taskId: approval.taskId,
            type: "approval",
            message: `Changes requested: ${feedback.slice(0, 200)}...`,
        });
        // Restart the agent with the feedback
        const agent = await (0, claude_code_js_1.startAgentForTask)(approval.taskId, feedback);
        if (!agent) {
            return {
                content: [
                    {
                        type: "text",
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
                    type: "text",
                    text: JSON.stringify({
                        success: true,
                        message: `Agent restarted with feedback. Task ${approval.taskId} is back in progress.`,
                    }),
                },
            ],
        };
    });
}
