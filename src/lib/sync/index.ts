import { db, workspaces, tasks, logs } from "@/lib/db";
import { fetchIssues, parseGitHubRemote } from "@/lib/github";
import { eq, and } from "drizzle-orm";

export async function syncWorkspaceIssues(workspaceId: number): Promise<{
  added: number;
  updated: number;
  errors: string[];
}> {
  const result = { added: 0, updated: 0, errors: [] as string[] };

  // Get workspace
  const workspace = await db.query.workspaces.findFirst({
    where: eq(workspaces.id, workspaceId),
  });

  if (!workspace?.githubRemote) {
    result.errors.push("Workspace has no GitHub remote configured");
    return result;
  }

  const repo = parseGitHubRemote(workspace.githubRemote);
  if (!repo) {
    result.errors.push(`Invalid GitHub remote: ${workspace.githubRemote}`);
    return result;
  }

  // Fetch issues from GitHub
  const issues = await fetchIssues(repo, "open");

  for (const issue of issues) {
    // Check if task already exists
    const existing = await db.query.tasks.findFirst({
      where: and(
        eq(tasks.workspaceId, workspaceId),
        eq(tasks.githubIssueNumber, issue.number)
      ),
    });

    if (existing) {
      // Update existing task if not already in progress
      if (existing.state === "new") {
        await db
          .update(tasks)
          .set({
            title: issue.title,
            description: issue.body,
            updatedAt: new Date(),
          })
          .where(eq(tasks.id, existing.id));
        result.updated++;
      }
    } else {
      // Create new task
      await db.insert(tasks).values({
        workspaceId,
        githubIssueNumber: issue.number,
        title: issue.title,
        description: issue.body,
        state: "new",
        priority: issue.labels.includes("priority:high") ? 1 : 0,
      });
      result.added++;
    }
  }

  // Log sync result
  await db.insert(logs).values({
    workspaceId,
    type: "sync",
    message: `Synced ${result.added} new, ${result.updated} updated issues`,
    details: JSON.stringify({ repo: repo.fullName, issues: issues.length }),
  });

  return result;
}

export async function syncAllWorkspaces(): Promise<{
  workspaces: number;
  added: number;
  updated: number;
  errors: string[];
}> {
  const result = { workspaces: 0, added: 0, updated: 0, errors: [] as string[] };

  const allWorkspaces = await db.select().from(workspaces);

  for (const workspace of allWorkspaces) {
    if (!workspace.githubRemote) continue;

    result.workspaces++;
    const syncResult = await syncWorkspaceIssues(workspace.id);
    result.added += syncResult.added;
    result.updated += syncResult.updated;
    result.errors.push(...syncResult.errors);
  }

  return result;
}
