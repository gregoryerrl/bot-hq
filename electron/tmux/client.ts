import { execSync } from 'child_process'

export class TmuxClient {
  private hasTmux: boolean

  constructor() {
    try {
      execSync('which tmux', { encoding: 'utf-8' })
      this.hasTmux = true
    } catch {
      this.hasTmux = false
    }
  }

  isAvailable(): boolean {
    return this.hasTmux
  }

  listSessions(): Array<{ name: string; windows: number; attached: boolean }> {
    if (!this.hasTmux) return []
    try {
      const output = execSync(
        'tmux list-sessions -F "#{session_name}|#{session_windows}|#{session_attached}"',
        { encoding: 'utf-8' }
      )
      return output
        .trim()
        .split('\n')
        .filter(Boolean)
        .map((line) => {
          const [name, windows, attached] = line.split('|')
          return { name, windows: parseInt(windows), attached: attached === '1' }
        })
    } catch {
      return []
    }
  }

  listPanes(): Array<{ target: string; pid: number; command: string; cwd: string }> {
    if (!this.hasTmux) return []
    try {
      const output = execSync(
        'tmux list-panes -a -F "#{session_name}:#{window_index}.#{pane_index}|#{pane_pid}|#{pane_current_command}|#{pane_current_path}"',
        { encoding: 'utf-8' }
      )
      return output
        .trim()
        .split('\n')
        .filter(Boolean)
        .map((line) => {
          const [target, pid, command, cwd] = line.split('|')
          return { target, pid: parseInt(pid), command, cwd }
        })
    } catch {
      return []
    }
  }

  sendKeys(target: string, keys: string): void {
    if (!this.hasTmux) throw new Error('tmux not available')
    const escaped = keys.replace(/'/g, "'\\''")
    execSync(`tmux send-keys -t '${target}' '${escaped}' Enter`, { encoding: 'utf-8' })
  }

  capturePane(target: string, lines = 50): string {
    if (!this.hasTmux) throw new Error('tmux not available')
    try {
      return execSync(`tmux capture-pane -t '${target}' -p -S -${lines}`, { encoding: 'utf-8' })
    } catch {
      return ''
    }
  }

  newWindow(sessionName: string, command: string, cwd?: string): string {
    if (!this.hasTmux) throw new Error('tmux not available')
    const cdPart = cwd ? `cd '${cwd}' && ` : ''
    try {
      execSync(`tmux has-session -t '${sessionName}' 2>/dev/null`)
      execSync(`tmux new-window -t '${sessionName}' '${cdPart}${command}'`)
      const windows = execSync(
        `tmux list-windows -t '${sessionName}' -F '#{window_index}'`,
        { encoding: 'utf-8' }
      )
      const lastWindow = windows.trim().split('\n').pop()
      return `${sessionName}:${lastWindow}.0`
    } catch {
      execSync(`tmux new-session -d -s '${sessionName}' '${cdPart}${command}'`)
      return `${sessionName}:0.0`
    }
  }

  killPane(target: string): void {
    if (!this.hasTmux) throw new Error('tmux not available')
    execSync(`tmux kill-pane -t '${target}'`)
  }
}
