import { Octokit } from "@octokit/rest";
import { GitHubIssue, SyncResult, PRResult } from "./types.js";

// Read JSON-RPC messages from stdin
let buffer = "";

process.stdin.setEncoding("utf8");
process.stdin.on("data", (chunk) => {
  buffer += chunk;
  processBuffer();
});

function processBuffer() {
  const lines = buffer.split("\n");
  buffer = lines.pop() || "";

  for (const line of lines) {
    if (!line.trim()) continue;
    try {
      const message = JSON.parse(line);
      handleMessage(message);
    } catch (e) {
      console.error("Failed to parse message:", e);
    }
  }
}

interface RpcMessage {
  id: string;
  method: string;
  params?: Record<string, unknown>;
}

async function handleMessage(message: RpcMessage) {
  const { id, method, params } = message;

  try {
    let result: unknown;

    if (method === "tools/call") {
      const { name, arguments: args } = params as { name: string; arguments: Record<string, unknown> };
      result = await callTool(name, args);
    } else if (method === "tools/list") {
      result = getToolsList();
    } else {
      throw new Error(`Unknown method: ${method}`);
    }

    respond(id, result);
  } catch (error) {
    respondError(id, error instanceof Error ? error.message : "Unknown error");
  }
}

function respond(id: string, result: unknown) {
  const response = JSON.stringify({ jsonrpc: "2.0", id, result });
  process.stdout.write(response + "\n");
}

function respondError(id: string, message: string) {
  const response = JSON.stringify({ jsonrpc: "2.0", id, error: { code: -1, message } });
  process.stdout.write(response + "\n");
}

function getToolsList() {
  return {
    tools: [
      {
        name: "github_sync_issues",
        description: "Sync GitHub issues to Bot-HQ tasks",
        inputSchema: {
          type: "object",
          properties: {
            owner: { type: "string", description: "Repository owner" },
            repo: { type: "string", description: "Repository name" },
            labels: { type: "array", items: { type: "string" }, description: "Filter by labels" },
          },
          required: ["owner", "repo"],
        },
      },
      {
        name: "github_create_pr",
        description: "Create a pull request from a branch",
        inputSchema: {
          type: "object",
          properties: {
            owner: { type: "string" },
            repo: { type: "string" },
            head: { type: "string", description: "Branch to create PR from" },
            base: { type: "string", description: "Target branch" },
            title: { type: "string" },
            body: { type: "string" },
          },
          required: ["owner", "repo", "head", "base", "title"],
        },
      },
      {
        name: "github_push_branch",
        description: "Push a local branch to remote",
        inputSchema: {
          type: "object",
          properties: {
            repoPath: { type: "string", description: "Local repository path" },
            branch: { type: "string", description: "Branch name to push" },
            remote: { type: "string", description: "Remote name", default: "origin" },
          },
          required: ["repoPath", "branch"],
        },
      },
      {
        name: "github_get_issue",
        description: "Get details of a GitHub issue",
        inputSchema: {
          type: "object",
          properties: {
            owner: { type: "string" },
            repo: { type: "string" },
            issueNumber: { type: "number" },
          },
          required: ["owner", "repo", "issueNumber"],
        },
      },
    ],
  };
}

async function callTool(name: string, args: Record<string, unknown>): Promise<unknown> {
  const token = process.env.GITHUB_TOKEN;
  if (!token) {
    throw new Error("GITHUB_TOKEN not configured");
  }

  const octokit = new Octokit({ auth: token });

  switch (name) {
    case "github_sync_issues":
      return syncIssues(octokit, args as { owner: string; repo: string; labels?: string[] });
    case "github_create_pr":
      return createPR(octokit, args as { owner: string; repo: string; head: string; base: string; title: string; body?: string });
    case "github_push_branch":
      return pushBranch(args as { repoPath: string; branch: string; remote?: string });
    case "github_get_issue":
      return getIssue(octokit, args as { owner: string; repo: string; issueNumber: number });
    default:
      throw new Error(`Unknown tool: ${name}`);
  }
}

async function syncIssues(
  octokit: Octokit,
  { owner, repo, labels }: { owner: string; repo: string; labels?: string[] }
): Promise<SyncResult> {
  const { data: issues } = await octokit.issues.listForRepo({
    owner,
    repo,
    state: "open",
    labels: labels?.join(","),
    per_page: 100,
  });

  const result: GitHubIssue[] = issues
    .filter(issue => !issue.pull_request) // Exclude PRs
    .map(issue => ({
      number: issue.number,
      title: issue.title,
      body: issue.body,
      state: issue.state,
      labels: issue.labels.map(l => (typeof l === "string" ? l : l.name || "")),
      assignees: issue.assignees?.map(a => a.login) || [],
      url: issue.html_url,
    }));

  return {
    synced: result.length,
    created: 0,
    updated: 0,
    issues: result,
  };
}

async function createPR(
  octokit: Octokit,
  { owner, repo, head, base, title, body }: { owner: string; repo: string; head: string; base: string; title: string; body?: string }
): Promise<PRResult> {
  const { data: pr } = await octokit.pulls.create({
    owner,
    repo,
    head,
    base,
    title,
    body: body || "",
  });

  return {
    number: pr.number,
    url: pr.html_url,
    title: pr.title,
  };
}

async function pushBranch(
  { repoPath, branch, remote = "origin" }: { repoPath: string; branch: string; remote?: string }
): Promise<{ success: boolean; output: string }> {
  const { exec } = await import("child_process");
  const { promisify } = await import("util");
  const execAsync = promisify(exec);

  const { stdout, stderr } = await execAsync(
    `git push -u ${remote} ${branch}`,
    { cwd: repoPath }
  );

  return {
    success: true,
    output: stdout || stderr,
  };
}

async function getIssue(
  octokit: Octokit,
  { owner, repo, issueNumber }: { owner: string; repo: string; issueNumber: number }
): Promise<GitHubIssue> {
  const { data: issue } = await octokit.issues.get({
    owner,
    repo,
    issue_number: issueNumber,
  });

  return {
    number: issue.number,
    title: issue.title,
    body: issue.body,
    state: issue.state,
    labels: issue.labels.map(l => (typeof l === "string" ? l : l.name || "")),
    assignees: issue.assignees?.map(a => a.login) || [],
    url: issue.html_url,
  };
}

// Keep process alive
process.stdin.resume();
console.error("[github-plugin] MCP server started");
