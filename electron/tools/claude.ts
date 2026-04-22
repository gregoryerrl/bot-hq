import { ToolDefinition } from './types'
import { ClaudeSessionManager } from '../tmux/session-manager'
import { execSync } from 'child_process'

const sessionManager = new ClaudeSessionManager()

const EXEC_OPTIONS = {
  encoding: 'utf-8' as const,
  timeout: 120000,
  maxBuffer: 5 * 1024 * 1024
}

const MAX_OUTPUT = 30000

function truncate(output: string): string {
  if (output.length > MAX_OUTPUT) {
    return output.slice(0, MAX_OUTPUT) + '\n...[truncated]'
  }
  return output
}

function escapePrompt(prompt: string): string {
  return prompt.replace(/"/g, '\\"')
}

export const claudeTools: ToolDefinition[] = [
  {
    name: 'claude_send',
    description: 'Send a one-shot task to Claude Code using --print mode. Returns the full output. Good for quick questions or tasks that do not need an ongoing session.',
    parameters: {
      type: 'OBJECT',
      properties: {
        prompt: { type: 'STRING', description: 'The prompt to send to Claude Code' },
        cwd: { type: 'STRING', description: 'Working directory for the command (optional, defaults to home directory)' }
      },
      required: ['prompt']
    },
    destructive: false,
    execute: async (args) => {
      const prompt = args.prompt as string
      const cwd = (args.cwd as string) || process.env.HOME || '/'

      try {
        const output = execSync(`claude --print -p "${escapePrompt(prompt)}"`, {
          ...EXEC_OPTIONS,
          cwd
        })
        return { output: truncate(output.trim()) }
      } catch (err: unknown) {
        const error = err as { stdout?: string; stderr?: string; message?: string }
        if (error.stdout) {
          return { output: truncate(error.stdout.trim()), partial: true }
        }
        return { error: error.message || 'claude --print failed' }
      }
    }
  },

  {
    name: 'claude_start',
    description: 'Start a new persistent Claude Code session in a tmux window. The session runs with --dangerously-skip-permissions and can be interacted with via claude_message and claude_read.',
    parameters: {
      type: 'OBJECT',
      properties: {
        project_path: { type: 'STRING', description: 'Absolute path to the project directory where Claude Code should run' }
      },
      required: ['project_path']
    },
    destructive: false,
    execute: async (args) => {
      const projectPath = args.project_path as string

      try {
        const result = sessionManager.startSession(projectPath)
        return { success: true, sessionId: result.id, tmuxTarget: result.tmuxTarget }
      } catch (err: unknown) {
        const error = err as { message?: string }
        return { error: error.message || 'Failed to start Claude session' }
      }
    }
  },

  {
    name: 'claude_message',
    description: 'Send a message to a running Claude Code session. The message is typed into the tmux pane and recent output is captured and returned.',
    parameters: {
      type: 'OBJECT',
      properties: {
        session_id: { type: 'STRING', description: 'The session ID to send the message to' },
        message: { type: 'STRING', description: 'The message to send to Claude Code' }
      },
      required: ['session_id', 'message']
    },
    destructive: false,
    execute: async (args) => {
      const sessionId = args.session_id as string
      const message = args.message as string

      try {
        const output = sessionManager.sendMessage(sessionId, message)
        return { output: truncate(output) }
      } catch (err: unknown) {
        const error = err as { message?: string }
        return { error: error.message || 'Failed to send message' }
      }
    }
  },

  {
    name: 'claude_read',
    description: 'Read the latest output from a running Claude Code session. Captures the current tmux pane content.',
    parameters: {
      type: 'OBJECT',
      properties: {
        session_id: { type: 'STRING', description: 'The session ID to read output from' }
      },
      required: ['session_id']
    },
    destructive: false,
    execute: async (args) => {
      const sessionId = args.session_id as string

      try {
        const output = sessionManager.readOutput(sessionId)
        return { output: truncate(output) }
      } catch (err: unknown) {
        const error = err as { message?: string }
        return { error: error.message || 'Failed to read session output' }
      }
    }
  },

  {
    name: 'claude_list',
    description: 'List all running Claude Code sessions discovered in tmux. Returns session metadata including IDs, project paths, and status.',
    parameters: {
      type: 'OBJECT',
      properties: {}
    },
    destructive: false,
    execute: async () => {
      try {
        const sessions = sessionManager.discover()
        return { sessions }
      } catch (err: unknown) {
        const error = err as { message?: string }
        return { error: error.message || 'Failed to list sessions' }
      }
    }
  },

  {
    name: 'claude_attach',
    description: 'Discover and adopt running Claude Code sessions found in tmux. Sessions are registered in the database for management via other claude_* tools.',
    parameters: {
      type: 'OBJECT',
      properties: {}
    },
    destructive: false,
    execute: async () => {
      try {
        const sessions = sessionManager.discover()
        return { attached: sessions.length, sessions }
      } catch (err: unknown) {
        const error = err as { message?: string }
        return { error: error.message || 'Failed to attach sessions' }
      }
    }
  },

  {
    name: 'claude_stop',
    description: 'Stop a running Claude Code session by killing its tmux pane. This is destructive and cannot be undone.',
    parameters: {
      type: 'OBJECT',
      properties: {
        session_id: { type: 'STRING', description: 'The session ID to stop' }
      },
      required: ['session_id']
    },
    destructive: true,
    execute: async (args) => {
      const sessionId = args.session_id as string

      try {
        sessionManager.stopSession(sessionId)
        return { success: true, sessionId }
      } catch (err: unknown) {
        const error = err as { message?: string }
        return { error: error.message || 'Failed to stop session' }
      }
    }
  },

  {
    name: 'claude_continue',
    description: 'Continue the last Claude Code conversation using -c flag. Resumes the most recent session in the given working directory.',
    parameters: {
      type: 'OBJECT',
      properties: {
        prompt: { type: 'STRING', description: 'The prompt to continue the conversation with' },
        cwd: { type: 'STRING', description: 'Working directory (optional, defaults to home directory)' }
      },
      required: ['prompt']
    },
    destructive: false,
    execute: async (args) => {
      const prompt = args.prompt as string
      const cwd = (args.cwd as string) || process.env.HOME || '/'

      try {
        const output = execSync(`claude -c --print -p "${escapePrompt(prompt)}"`, {
          ...EXEC_OPTIONS,
          cwd
        })
        return { output: truncate(output.trim()) }
      } catch (err: unknown) {
        const error = err as { stdout?: string; stderr?: string; message?: string }
        if (error.stdout) {
          return { output: truncate(error.stdout.trim()), partial: true }
        }
        return { error: error.message || 'claude -c --print failed' }
      }
    }
  }
]
