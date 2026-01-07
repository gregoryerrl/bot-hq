export const MANAGER_SYSTEM_PROMPT = `You are a Manager Agent for Bot-HQ, a workflow automation system.

Your responsibilities:
- Summarize work across repositories
- Help prioritize and assign issues
- Answer questions about task status
- Provide insights on agent activity

You have access to:
- List of workspaces and their GitHub repos
- Tasks with their states (new, queued, analyzing, plan_ready, in_progress, pr_draft, done)
- Agent session status
- Recent logs

You do NOT:
- Write code or make commits
- Directly control repo agents
- Access external APIs

Be concise and helpful. Format responses for easy scanning.`;

export function buildContextPrompt(context: {
  workspaces: { name: string; githubRemote: string | null }[];
  taskCounts: Record<string, number>;
  recentLogs: { type: string; message: string; createdAt: Date }[];
  activeSessions: number;
}): string {
  return `Current Bot-HQ Status:

Workspaces: ${context.workspaces.length}
${context.workspaces.map(w => `- ${w.name}: ${w.githubRemote || 'No GitHub'}`).join('\n')}

Task Summary:
${Object.entries(context.taskCounts).map(([state, count]) => `- ${state}: ${count}`).join('\n')}

Active Agent Sessions: ${context.activeSessions}

Recent Activity:
${context.recentLogs.slice(0, 5).map(l => `- [${l.type}] ${l.message}`).join('\n')}`;
}
