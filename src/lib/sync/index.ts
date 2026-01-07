import { db, workspaces, tasks, logs } from "@/lib/db";
import { fetchIssues, parseGitHubRemote } from "@/lib/github";
import { eq, and } from "drizzle-orm";

export async function syncWorkspaceIssues(workspaceId: number): Promise<{
  added: number;
  updated: number;
  closed: number;
  errors: string[];
}> {
  const result = { added: 0, updated: 0, closed: 0, errors: [] as string[] };

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

  // Fetch both open and closed issues
  const [openIssues, closedIssues] = await Promise.all([
    fetchIssues(repo, "open"),
    fetchIssues(repo, "closed"),
  ]);

  // Process open issues
  for (const issue of openIssues) {
    const existing = await db.query.tasks.findFirst({
      where: and(
        eq(tasks.workspaceId, workspaceId),
        eq(tasks.githubIssueNumber, issue.number)
      ),
    });

    if (existing) {
      // Update existing task if not actively being worked on
      const canUpdate = ["new", "queued"].includes(existing.state);
      if (canUpdate) {
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

  // Mark tasks as done if their GitHub issue is closed
  for (const issue of closedIssues) {
    const existing = await db.query.tasks.findFirst({
      where: and(
        eq(tasks.workspaceId, workspaceId),
        eq(tasks.githubIssueNumber, issue.number)
      ),
    });

    if (existing && existing.state !== "done") {
      await db
        .update(tasks)
        .set({
          state: "done",
          updatedAt: new Date(),
        })
        .where(eq(tasks.id, existing.id));
      result.closed++;
    }
  }

  // Log sync result
  await db.insert(logs).values({
    workspaceId,
    type: "sync",
    message: `Synced ${result.added} new, ${result.updated} updated, ${result.closed} closed issues`,
    details: JSON.stringify({ repo: repo.fullName, open: openIssues.length, closed: closedIssues.length }),
  });

  return result;
}

export async function syncAllWorkspaces(): Promise<{
  workspaces: number;
  added: number;
  updated: number;
  closed: number;
  errors: string[];
}> {
  const result = { workspaces: 0, added: 0, updated: 0, closed: 0, errors: [] as string[] };

  const allWorkspaces = await db.select().from(workspaces);

  for (const workspace of allWorkspaces) {
    if (!workspace.githubRemote) continue;

    result.workspaces++;
    const syncResult = await syncWorkspaceIssues(workspace.id);
    result.added += syncResult.added;
    result.updated += syncResult.updated;
    result.closed += syncResult.closed;
    result.errors.push(...syncResult.errors);
  }

  return result;
}
