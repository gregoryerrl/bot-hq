import { ToolDefinition } from './types'
import { execSync, exec } from 'child_process'
import * as os from 'os'

export const shellTools: ToolDefinition[] = [
  {
    name: 'run_command',
    description: 'Execute a shell command and return its output. Uses /bin/zsh.',
    parameters: {
      type: 'OBJECT',
      properties: {
        command: { type: 'STRING', description: 'The shell command to execute' },
        cwd: { type: 'STRING', description: 'Working directory for the command (optional)' },
        timeout_ms: { type: 'STRING', description: 'Timeout in milliseconds (default 30000)' }
      },
      required: ['command']
    },
    destructive: false,
    execute: async (args) => {
      const command = args.command as string
      const cwd = (args.cwd as string) || process.cwd()
      const timeout = parseInt(args.timeout_ms as string, 10) || 30000

      try {
        const stdout = execSync(command, {
          cwd,
          timeout,
          encoding: 'utf-8',
          shell: '/bin/zsh',
          maxBuffer: 1024 * 1024 * 5,
          stdio: ['pipe', 'pipe', 'pipe']
        })
        return { stdout: stdout.trim(), stderr: '' }
      } catch (err: unknown) {
        const error = err as { stdout?: string; stderr?: string; message?: string; status?: number }
        if (error.stdout !== undefined || error.stderr !== undefined) {
          return {
            stdout: (error.stdout || '').trim(),
            stderr: (error.stderr || '').trim(),
            exitCode: error.status ?? 1
          }
        }
        return { error: error.message || 'Command execution failed' }
      }
    }
  },

  {
    name: 'open_app',
    description: 'Open a macOS application by name using the open command',
    parameters: {
      type: 'OBJECT',
      properties: {
        name: { type: 'STRING', description: 'Name of the application to open (e.g. "Safari", "Finder")' }
      },
      required: ['name']
    },
    destructive: false,
    execute: async (args) => {
      const name = args.name as string
      try {
        execSync(`open -a "${name.replace(/"/g, '\\"')}"`, {
          encoding: 'utf-8',
          timeout: 10000
        })
        return { success: true, app: name }
      } catch (err: unknown) {
        const error = err as { message?: string }
        return { error: error.message || `Failed to open ${name}` }
      }
    }
  },

  {
    name: 'kill_process',
    description: 'Kill a process by name or PID',
    parameters: {
      type: 'OBJECT',
      properties: {
        name: { type: 'STRING', description: 'Process name to kill (uses pkill)' },
        pid: { type: 'STRING', description: 'Process ID to kill' }
      }
    },
    destructive: true,
    execute: async (args) => {
      const name = args.name as string | undefined
      const pid = args.pid as string | undefined

      if (!name && !pid) {
        return { error: 'Either name or pid must be provided' }
      }

      try {
        if (pid) {
          execSync(`kill ${parseInt(pid, 10)}`, { encoding: 'utf-8', timeout: 5000 })
          return { success: true, killed: `PID ${pid}` }
        } else {
          execSync(`pkill -f "${name!.replace(/"/g, '\\"')}"`, { encoding: 'utf-8', timeout: 5000 })
          return { success: true, killed: name }
        }
      } catch (err: unknown) {
        const error = err as { status?: number; message?: string }
        if (error.status === 1) {
          return { error: 'No matching process found' }
        }
        return { error: error.message || 'Failed to kill process' }
      }
    }
  },

  {
    name: 'system_info',
    description: 'Get system information including CPU, memory, disk usage, and uptime',
    parameters: {
      type: 'OBJECT',
      properties: {}
    },
    destructive: false,
    execute: async () => {
      const cpus = os.cpus()
      const totalMem = os.totalmem()
      const freeMem = os.freemem()

      let diskInfo = ''
      try {
        diskInfo = execSync("df -h / | tail -1 | awk '{print $2, $3, $4, $5}'", {
          encoding: 'utf-8',
          timeout: 5000
        }).trim()
      } catch {
        diskInfo = 'unavailable'
      }

      const [diskTotal, diskUsed, diskAvail, diskPercent] = diskInfo.split(' ')

      return {
        hostname: os.hostname(),
        platform: os.platform(),
        arch: os.arch(),
        uptime_hours: Math.round(os.uptime() / 3600 * 10) / 10,
        cpu: {
          model: cpus[0]?.model || 'unknown',
          cores: cpus.length
        },
        memory: {
          total_gb: Math.round(totalMem / (1024 ** 3) * 10) / 10,
          free_gb: Math.round(freeMem / (1024 ** 3) * 10) / 10,
          used_percent: Math.round((1 - freeMem / totalMem) * 100)
        },
        disk: {
          total: diskTotal || 'unknown',
          used: diskUsed || 'unknown',
          available: diskAvail || 'unknown',
          used_percent: diskPercent || 'unknown'
        }
      }
    }
  },

  {
    name: 'list_processes',
    description: 'List running processes, optionally filtered by name',
    parameters: {
      type: 'OBJECT',
      properties: {
        name: { type: 'STRING', description: 'Optional filter to match process names (case-insensitive)' }
      }
    },
    destructive: false,
    execute: async (args) => {
      const nameFilter = args.name as string | undefined

      try {
        const output = execSync('ps aux', {
          encoding: 'utf-8',
          timeout: 5000,
          maxBuffer: 1024 * 1024 * 5
        })

        const lines = output.trim().split('\n')
        const header = lines[0]
        let processLines = lines.slice(1)

        if (nameFilter) {
          const filter = nameFilter.toLowerCase()
          processLines = processLines.filter(line => line.toLowerCase().includes(filter))
        }

        const processes = processLines.slice(0, 100).map(line => {
          const parts = line.trim().split(/\s+/)
          return {
            user: parts[0],
            pid: parts[1],
            cpu: parts[2],
            mem: parts[3],
            command: parts.slice(10).join(' ')
          }
        })

        return { count: processes.length, processes }
      } catch (err: unknown) {
        const error = err as { message?: string }
        return { error: error.message || 'Failed to list processes' }
      }
    }
  }
]
