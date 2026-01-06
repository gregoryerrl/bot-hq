export interface GitHubIssue {
  number: number;
  title: string;
  body: string;
  state: "open" | "closed";
  labels: string[];
  assignees: string[];
  createdAt: string;
  updatedAt: string;
  url: string;
}

export interface GitHubRepo {
  owner: string;
  name: string;
  fullName: string;
}

export function parseGitHubRemote(remote: string): GitHubRepo | null {
  // Handles: owner/repo, https://github.com/owner/repo, git@github.com:owner/repo
  const patterns = [
    /^([^/]+)\/([^/]+)$/,
    /github\.com\/([^/]+)\/([^/]+?)(?:\.git)?$/,
    /github\.com:([^/]+)\/([^/]+?)(?:\.git)?$/,
  ];

  for (const pattern of patterns) {
    const match = remote.match(pattern);
    if (match) {
      return {
        owner: match[1],
        name: match[2],
        fullName: `${match[1]}/${match[2]}`,
      };
    }
  }
  return null;
}
