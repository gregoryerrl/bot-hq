import { TmuxClient } from './client'
import { execSync } from 'child_process'

export interface DiscoveredSession {
  tmuxTarget: string
  pid: number
  cwd: string
}

export function discoverClaudeSessions(tmux: TmuxClient): DiscoveredSession[] {
  const panes = tmux.listPanes()
  const sessions: DiscoveredSession[] = []

  for (const pane of panes) {
    try {
      const children = execSync(`pgrep -P ${pane.pid} -l 2>/dev/null || true`, { encoding: 'utf-8' })
      const paneOutput = tmux.capturePane(pane.target, 5)
      const isClaudeSession =
        pane.command.includes('claude') ||
        children.includes('claude') ||
        paneOutput.includes('claude') ||
        paneOutput.includes('Claude Code')

      if (isClaudeSession) {
        const claudePid = findClaudePid(pane.pid)
        sessions.push({
          tmuxTarget: pane.target,
          pid: claudePid || pane.pid,
          cwd: pane.cwd
        })
      }
    } catch {
      // Skip panes that fail inspection
    }
  }

  return sessions
}

function findClaudePid(parentPid: number): number | null {
  try {
    const output = execSync(`pgrep -P ${parentPid} -f claude 2>/dev/null | head -1`, {
      encoding: 'utf-8'
    }).trim()
    return output ? parseInt(output) : null
  } catch {
    return null
  }
}
