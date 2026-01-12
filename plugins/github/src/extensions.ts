import type { PluginContext, PluginExtensions, ActionContext, ActionResult } from "./plugin-types.js";

export default function(ctx: PluginContext): PluginExtensions {
  return {
    actions: {
      approval: [
        {
          id: "create-pr",
          label: "Create Pull Request",
          description: (context: ActionContext) => {
            const workspace = context.workspace;
            if (!workspace) return "Push branch and create PR on GitHub";
            return `Push to GitHub and create PR`;
          },
          icon: "git-pull-request",
          defaultChecked: true,
          handler: async (context: ActionContext): Promise<ActionResult> => {
            const { approval, task, workspace, pluginContext } = context;

            if (!approval || !workspace) {
              return { success: false, error: "Missing approval or workspace context" };
            }

            // Get GitHub config from workspace data
            const workspaceData = await pluginContext.workspaceData.get(workspace.id) as { owner?: string; repo?: string } | null;

            if (!workspaceData?.owner || !workspaceData?.repo) {
              return { success: false, error: "GitHub not configured for this workspace. Go to Settings > Workspaces to configure." };
            }

            try {
              // First push the branch
              await pluginContext.mcp.call("github_push_branch", {
                repoPath: workspace.repoPath,
                branch: approval.branchName,
              });

              // Then create the PR
              const result = await pluginContext.mcp.call("github_create_pr", {
                owner: workspaceData.owner,
                repo: workspaceData.repo,
                head: approval.branchName,
                base: approval.baseBranch,
                title: task?.title || approval.branchName,
                body: `## Summary\n\n${task?.description || "No description"}\n\n---\nCreated by Bot-HQ`,
              }) as { url: string; number: number };

              // Store PR URL in task data
              if (task) {
                await pluginContext.taskData.set(task.id, {
                  prNumber: result.number,
                  prUrl: result.url,
                });
              }

              return {
                success: true,
                message: `PR #${result.number} created`,
                data: result,
              };
            } catch (error) {
              return {
                success: false,
                error: error instanceof Error ? error.message : "Failed to create PR",
              };
            }
          },
        },
      ],

      task: [
        {
          id: "view-on-github",
          label: "View on GitHub",
          description: "Open the linked GitHub issue or PR",
          icon: "external-link",
          handler: async (context: ActionContext): Promise<ActionResult> => {
            const { task, pluginContext } = context;

            if (!task) {
              return { success: false, error: "No task context" };
            }

            const taskData = await pluginContext.taskData.get(task.id) as { issueUrl?: string; prUrl?: string } | null;

            if (taskData?.prUrl) {
              return { success: true, message: "Opening PR", data: { url: taskData.prUrl, action: "open" } };
            }

            if (taskData?.issueUrl) {
              return { success: true, message: "Opening issue", data: { url: taskData.issueUrl, action: "open" } };
            }

            return { success: false, error: "No GitHub link for this task" };
          },
        },
      ],
    },

    hooks: {
      onTaskCreated: async (task) => {
        ctx.log.info(`Task created: ${task.title}`);
      },

      onApprovalAccepted: async (approval, task) => {
        ctx.log.info(`Approval accepted for task: ${task.title}`);
      },
    },
  };
}
