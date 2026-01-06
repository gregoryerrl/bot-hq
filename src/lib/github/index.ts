import { exec } from "child_process";
import { promisify } from "util";
import { GitHubIssue, GitHubRepo, parseGitHubRemote } from "./types";

const execAsync = promisify(exec);

export async function checkGhAuth(): Promise<boolean> {
  try {
    await execAsync("gh auth status");
    return true;
  } catch {
    return false;
  }
}

export async function fetchIssues(
  repo: GitHubRepo,
  state: "open" | "closed" | "all" = "open"
): Promise<GitHubIssue[]> {
  try {
    const { stdout } = await execAsync(
      `gh issue list --repo ${repo.fullName} --state ${state} --json number,title,body,state,labels,assignees,createdAt,updatedAt,url --limit 100`
    );

    const issues = JSON.parse(stdout);
    return issues.map((issue: Record<string, unknown>) => ({
      number: issue.number,
      title: issue.title,
      body: issue.body || "",
      state: issue.state,
      labels: (issue.labels as Array<{ name: string }>)?.map((l) => l.name) || [],
      assignees: (issue.assignees as Array<{ login: string }>)?.map((a) => a.login) || [],
      createdAt: issue.createdAt,
      updatedAt: issue.updatedAt,
      url: issue.url,
    }));
  } catch (error) {
    console.error(`Failed to fetch issues for ${repo.fullName}:`, error);
    return [];
  }
}

export async function createLinkedBranch(
  repo: GitHubRepo,
  issueNumber: number,
  cwd: string
): Promise<string | null> {
  try {
    // gh issue develop creates a branch linked to the issue
    const { stdout } = await execAsync(
      `gh issue develop ${issueNumber} --repo ${repo.fullName} --checkout`,
      { cwd }
    );
    // Extract branch name from output
    const match = stdout.match(/Switched to.*branch '([^']+)'/);
    return match ? match[1] : `${issueNumber}-issue`;
  } catch (error) {
    console.error(`Failed to create linked branch:`, error);
    return null;
  }
}

export async function createDraftPR(
  repo: GitHubRepo,
  branch: string,
  title: string,
  body: string,
  cwd: string
): Promise<string | null> {
  try {
    const { stdout } = await execAsync(
      `gh pr create --repo ${repo.fullName} --head ${branch} --title "${title.replace(/"/g, '\\"')}" --body "${body.replace(/"/g, '\\"')}" --draft`,
      { cwd }
    );
    // Extract PR URL from output
    const match = stdout.match(/(https:\/\/github\.com\/[^\s]+)/);
    return match ? match[1] : null;
  } catch (error) {
    console.error(`Failed to create draft PR:`, error);
    return null;
  }
}

export { parseGitHubRemote, type GitHubIssue, type GitHubRepo };
