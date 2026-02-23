import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { db, workspaces, gitRemotes, logs } from "../../lib/db/index.js";
import { eq } from "drizzle-orm";
import { exec } from "child_process";
import { promisify } from "util";

const execAsync = promisify(exec);

async function runGit(repoPath: string, args: string[]): Promise<string> {
  const expanded = repoPath.replace("~", process.env.HOME || "");
  try {
    const { stdout } = await execAsync(`git ${args.join(" ")}`, {
      cwd: expanded,
      maxBuffer: 5 * 1024 * 1024,
    });
    return stdout.trim();
  } catch (error: unknown) {
    const err = error as { stderr?: string; message?: string };
    return err.stderr || err.message || "unknown error";
  }
}

export function registerGitHealthTools(server: McpServer) {
  server.tool(
    "git_health_check",
    "Run a git health audit on a workspace repo — status, remotes, branches, recent commits, stash",
    {
      workspaceId: z.number().describe("The workspace ID to check"),
    },
    async ({ workspaceId }) => {
      const workspace = await db.query.workspaces.findFirst({
        where: eq(workspaces.id, workspaceId),
      });

      if (!workspace) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({ error: `Workspace ${workspaceId} not found` }),
            },
          ],
        };
      }

      const repoPath = workspace.repoPath;

      const [status, remotes, branches, recentLog, stashList] = await Promise.all([
        runGit(repoPath, ["status", "--porcelain"]),
        runGit(repoPath, ["remote", "-v"]),
        runGit(repoPath, ["branch", "-a", "--format=%(refname:short)"]),
        runGit(repoPath, ["log", "--oneline", "-5"]),
        runGit(repoPath, ["stash", "list"]),
      ]);

      // Detect stale task branches
      const branchList = branches.split("\n").filter(Boolean);
      const staleBranches = branchList.filter(
        (b) => b.startsWith("task/") || b.startsWith("origin/task/")
      );

      const summary = {
        workspaceId,
        workspaceName: workspace.name,
        repoPath,
        clean: status === "",
        dirtyFiles: status ? status.split("\n").length : 0,
        hasRemotes: remotes !== "",
        remotesOutput: remotes,
        totalBranches: branchList.length,
        staleBranches,
        recentCommits: recentLog,
        stashCount: stashList ? stashList.split("\n").length : 0,
      };

      // Write health log
      await db.insert(logs).values({
        workspaceId,
        type: "health",
        message: `Git health: ${summary.clean ? "clean" : `${summary.dirtyFiles} dirty files`}, ${summary.totalBranches} branches, ${summary.staleBranches.length} stale task branches`,
        details: JSON.stringify(summary),
      });

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify(summary, null, 2),
          },
        ],
      };
    }
  );

  server.tool(
    "git_remotes_check",
    "Check connectivity of all configured git remotes (tests GitHub API for GitHub remotes with credentials)",
    {},
    async () => {
      const allRemotes = await db.select().from(gitRemotes);

      if (allRemotes.length === 0) {
        return {
          content: [
            {
              type: "text" as const,
              text: JSON.stringify({
                message: "No git remotes configured",
                remotes: [],
              }),
            },
          ],
        };
      }

      const results = await Promise.all(
        allRemotes.map(async (remote) => {
          const workspace = remote.workspaceId
            ? await db.query.workspaces.findFirst({
                where: eq(workspaces.id, remote.workspaceId),
              })
            : null;

          let connectivity = "unchecked";
          let missingConfig: string[] = [];

          if (!remote.owner) missingConfig.push("owner");
          if (!remote.repo) missingConfig.push("repo");
          if (!remote.credentials) missingConfig.push("credentials");

          // Test GitHub connectivity if credentials exist
          if (remote.provider === "github" && remote.credentials) {
            try {
              const creds = JSON.parse(remote.credentials);
              if (creds.token) {
                const res = await fetch("https://api.github.com/user", {
                  headers: { Authorization: `Bearer ${creds.token}` },
                });
                connectivity = res.ok ? "connected" : `error (${res.status})`;
              }
            } catch {
              connectivity = "error (invalid credentials)";
            }
          }

          return {
            id: remote.id,
            name: remote.name,
            provider: remote.provider,
            workspaceName: workspace?.name || "global",
            owner: remote.owner,
            repo: remote.repo,
            connectivity,
            missingConfig,
          };
        })
      );

      // Write health log
      const connected = results.filter((r) => r.connectivity === "connected").length;
      const errors = results.filter((r) => r.connectivity.startsWith("error")).length;

      await db.insert(logs).values({
        type: "health",
        message: `Remotes check: ${results.length} total, ${connected} connected, ${errors} errors`,
        details: JSON.stringify(results),
      });

      return {
        content: [
          {
            type: "text" as const,
            text: JSON.stringify(results, null, 2),
          },
        ],
      };
    }
  );
}
