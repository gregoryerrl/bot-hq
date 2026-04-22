import { ToolDefinition } from './types'
import simpleGit from 'simple-git'

export const gitTools: ToolDefinition[] = [
  {
    name: 'git_status',
    description: 'Show working tree status for a git repository',
    parameters: {
      type: 'OBJECT',
      properties: {
        cwd: { type: 'STRING', description: 'Path to the git repository' }
      },
      required: ['cwd']
    },
    destructive: false,
    execute: async (args) => {
      const git = simpleGit(args.cwd as string)
      const status = await git.status()
      return {
        current: status.current,
        tracking: status.tracking,
        ahead: status.ahead,
        behind: status.behind,
        staged: status.staged,
        modified: status.modified,
        not_added: status.not_added,
        deleted: status.deleted,
        renamed: status.renamed,
        conflicted: status.conflicted,
        created: status.created,
        isClean: status.isClean()
      }
    }
  },

  {
    name: 'git_diff',
    description: 'Show diffs for a git repository. Optionally show staged changes.',
    parameters: {
      type: 'OBJECT',
      properties: {
        cwd: { type: 'STRING', description: 'Path to the git repository' },
        staged: { type: 'BOOLEAN', description: 'Show staged changes instead of unstaged (default false)' }
      },
      required: ['cwd']
    },
    destructive: false,
    execute: async (args) => {
      const git = simpleGit(args.cwd as string)
      const staged = args.staged as boolean | undefined
      const diffArgs = staged ? ['--cached'] : []
      const diff = await git.diff(diffArgs)
      const truncated = diff.length > 30000 ? diff.slice(0, 30000) + '\n... (truncated)' : diff
      return { diff: truncated }
    }
  },

  {
    name: 'git_log',
    description: 'Show commit history for a git repository',
    parameters: {
      type: 'OBJECT',
      properties: {
        cwd: { type: 'STRING', description: 'Path to the git repository' },
        count: { type: 'NUMBER', description: 'Number of commits to show (default 10)' }
      },
      required: ['cwd']
    },
    destructive: false,
    execute: async (args) => {
      const git = simpleGit(args.cwd as string)
      const count = (args.count as number) || 10
      const log = await git.log({ maxCount: count })
      const commits = log.all.map((c) => ({
        hash: c.hash,
        date: c.date,
        message: c.message,
        author_name: c.author_name,
        author_email: c.author_email
      }))
      return { commits }
    }
  },

  {
    name: 'git_commit',
    description: 'Stage files and create a commit in a git repository',
    parameters: {
      type: 'OBJECT',
      properties: {
        cwd: { type: 'STRING', description: 'Path to the git repository' },
        message: { type: 'STRING', description: 'Commit message' },
        files: { type: 'STRING', description: 'Space-separated file paths to stage, or "." for all' }
      },
      required: ['cwd', 'message', 'files']
    },
    destructive: false,
    execute: async (args) => {
      const git = simpleGit(args.cwd as string)
      const message = args.message as string
      const files = args.files as string

      if (files === '.') {
        await git.add('.')
      } else {
        const fileList = files.split(/\s+/).filter(Boolean)
        await git.add(fileList)
      }

      const result = await git.commit(message)
      return {
        commit: result.commit,
        branch: result.branch,
        summary: {
          changes: result.summary.changes,
          insertions: result.summary.insertions,
          deletions: result.summary.deletions
        }
      }
    }
  },

  {
    name: 'git_push',
    description: 'Push commits to a remote repository',
    parameters: {
      type: 'OBJECT',
      properties: {
        cwd: { type: 'STRING', description: 'Path to the git repository' },
        remote: { type: 'STRING', description: 'Remote name (default "origin")' },
        branch: { type: 'STRING', description: 'Branch name to push (optional, uses current branch)' }
      },
      required: ['cwd']
    },
    destructive: true,
    execute: async (args) => {
      const git = simpleGit(args.cwd as string)
      const remote = (args.remote as string) || 'origin'
      const branch = args.branch as string | undefined

      const pushArgs = branch ? [remote, branch] : [remote]
      const result = await git.push(pushArgs)
      return {
        pushed: true,
        remote,
        branch: result.pushed?.[0]?.local || branch || 'current',
        remoteMessages: result.remoteMessages
      }
    }
  },

  {
    name: 'git_pull',
    description: 'Pull changes from a remote repository',
    parameters: {
      type: 'OBJECT',
      properties: {
        cwd: { type: 'STRING', description: 'Path to the git repository' }
      },
      required: ['cwd']
    },
    destructive: false,
    execute: async (args) => {
      const git = simpleGit(args.cwd as string)
      const result = await git.pull()
      return {
        summary: {
          changes: result.summary.changes,
          insertions: result.summary.insertions,
          deletions: result.summary.deletions
        },
        files: result.files,
        created: result.created,
        deleted: result.deleted
      }
    }
  },

  {
    name: 'git_branch',
    description: 'List, create, or switch branches in a git repository',
    parameters: {
      type: 'OBJECT',
      properties: {
        cwd: { type: 'STRING', description: 'Path to the git repository' },
        action: { type: 'STRING', description: 'Action to perform', enum: ['list', 'create', 'switch'] },
        name: { type: 'STRING', description: 'Branch name (required for create and switch)' }
      },
      required: ['cwd', 'action']
    },
    destructive: false,
    execute: async (args) => {
      const git = simpleGit(args.cwd as string)
      const action = args.action as string
      const name = args.name as string | undefined

      switch (action) {
        case 'list': {
          const branches = await git.branch()
          return {
            current: branches.current,
            all: branches.all,
            branches: Object.entries(branches.branches).map(([key, b]) => ({
              name: key,
              current: b.current,
              commit: b.commit,
              label: b.label
            }))
          }
        }
        case 'create': {
          if (!name) return { error: 'Branch name is required for create action' }
          await git.checkoutLocalBranch(name)
          return { success: true, created: name }
        }
        case 'switch': {
          if (!name) return { error: 'Branch name is required for switch action' }
          await git.checkout(name)
          return { success: true, switched: name }
        }
        default:
          return { error: `Unknown action: ${action}` }
      }
    }
  },

  {
    name: 'git_stash',
    description: 'Stash, pop, or list stashed changes in a git repository',
    parameters: {
      type: 'OBJECT',
      properties: {
        cwd: { type: 'STRING', description: 'Path to the git repository' },
        action: { type: 'STRING', description: 'Action to perform', enum: ['push', 'pop', 'list'] }
      },
      required: ['cwd', 'action']
    },
    destructive: false,
    execute: async (args) => {
      const git = simpleGit(args.cwd as string)
      const action = args.action as string

      switch (action) {
        case 'push': {
          const result = await git.stash(['push'])
          return { success: true, message: result }
        }
        case 'pop': {
          const result = await git.stash(['pop'])
          return { success: true, message: result }
        }
        case 'list': {
          const result = await git.stashList()
          const stashes = result.all.map((s) => ({
            hash: s.hash,
            date: s.date,
            message: s.message
          }))
          return { stashes }
        }
        default:
          return { error: `Unknown action: ${action}` }
      }
    }
  }
]
