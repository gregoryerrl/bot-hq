"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.syncWorkspaceIssues = syncWorkspaceIssues;
exports.syncAllWorkspaces = syncAllWorkspaces;
const db_1 = require("@/lib/db");
const github_1 = require("@/lib/github");
const drizzle_orm_1 = require("drizzle-orm");
async function syncWorkspaceIssues(workspaceId) {
    const result = { added: 0, updated: 0, closed: 0, errors: [] };
    // Get workspace
    const workspace = await db_1.db.query.workspaces.findFirst({
        where: (0, drizzle_orm_1.eq)(db_1.workspaces.id, workspaceId),
    });
    if (!workspace?.githubRemote) {
        result.errors.push("Workspace has no GitHub remote configured");
        return result;
    }
    const repo = (0, github_1.parseGitHubRemote)(workspace.githubRemote);
    if (!repo) {
        result.errors.push(`Invalid GitHub remote: ${workspace.githubRemote}`);
        return result;
    }
    // Fetch both open and closed issues
    const [openIssues, closedIssues] = await Promise.all([
        (0, github_1.fetchIssues)(repo, "open"),
        (0, github_1.fetchIssues)(repo, "closed"),
    ]);
    // Process open issues
    for (const issue of openIssues) {
        const existing = await db_1.db.query.tasks.findFirst({
            where: (0, drizzle_orm_1.and)((0, drizzle_orm_1.eq)(db_1.tasks.workspaceId, workspaceId), (0, drizzle_orm_1.eq)(db_1.tasks.githubIssueNumber, issue.number)),
        });
        if (existing) {
            // Update existing task if not actively being worked on
            const canUpdate = ["new", "queued"].includes(existing.state);
            if (canUpdate) {
                await db_1.db
                    .update(db_1.tasks)
                    .set({
                    title: issue.title,
                    description: issue.body,
                    updatedAt: new Date(),
                })
                    .where((0, drizzle_orm_1.eq)(db_1.tasks.id, existing.id));
                result.updated++;
            }
        }
        else {
            // Create new task
            await db_1.db.insert(db_1.tasks).values({
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
        const existing = await db_1.db.query.tasks.findFirst({
            where: (0, drizzle_orm_1.and)((0, drizzle_orm_1.eq)(db_1.tasks.workspaceId, workspaceId), (0, drizzle_orm_1.eq)(db_1.tasks.githubIssueNumber, issue.number)),
        });
        if (existing && existing.state !== "done") {
            await db_1.db
                .update(db_1.tasks)
                .set({
                state: "done",
                updatedAt: new Date(),
            })
                .where((0, drizzle_orm_1.eq)(db_1.tasks.id, existing.id));
            result.closed++;
        }
    }
    // Log sync result
    await db_1.db.insert(db_1.logs).values({
        workspaceId,
        type: "sync",
        message: `Synced ${result.added} new, ${result.updated} updated, ${result.closed} closed issues`,
        details: JSON.stringify({ repo: repo.fullName, open: openIssues.length, closed: closedIssues.length }),
    });
    return result;
}
async function syncAllWorkspaces() {
    const result = { workspaces: 0, added: 0, updated: 0, closed: 0, errors: [] };
    const allWorkspaces = await db_1.db.select().from(db_1.workspaces);
    for (const workspace of allWorkspaces) {
        if (!workspace.githubRemote)
            continue;
        result.workspaces++;
        const syncResult = await syncWorkspaceIssues(workspace.id);
        result.added += syncResult.added;
        result.updated += syncResult.updated;
        result.closed += syncResult.closed;
        result.errors.push(...syncResult.errors);
    }
    return result;
}
