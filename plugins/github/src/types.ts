export interface GitHubConfig {
  owner: string;
  repo: string;
  defaultBranch?: string;
}

export interface GitHubIssue {
  number: number;
  title: string;
  body: string | null;
  state: string;
  labels: string[];
  assignees: string[];
  url: string;
}

export interface SyncResult {
  synced: number;
  created: number;
  updated: number;
  issues: GitHubIssue[];
}

export interface PRResult {
  number: number;
  url: string;
  title: string;
}
