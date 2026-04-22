import { ToolDefinition } from './types'
import * as fs from 'fs/promises'
import * as path from 'path'
import { execSync } from 'child_process'

export const fileTools: ToolDefinition[] = [
  {
    name: 'read_file',
    description: 'Read the contents of a file at the given path',
    parameters: {
      type: 'OBJECT',
      properties: {
        path: { type: 'STRING', description: 'Absolute or relative path to the file to read' }
      },
      required: ['path']
    },
    destructive: false,
    execute: async (args) => {
      const filePath = path.resolve(args.path as string)
      const content = await fs.readFile(filePath, 'utf-8')
      const truncated = content.length > 50000 ? content.slice(0, 50000) : content
      return { content: truncated }
    }
  },

  {
    name: 'write_file',
    description: 'Create or overwrite a file with the given content',
    parameters: {
      type: 'OBJECT',
      properties: {
        path: { type: 'STRING', description: 'Absolute or relative path to the file to write' },
        content: { type: 'STRING', description: 'Content to write to the file' }
      },
      required: ['path', 'content']
    },
    destructive: false,
    execute: async (args) => {
      const filePath = path.resolve(args.path as string)
      await fs.mkdir(path.dirname(filePath), { recursive: true })
      await fs.writeFile(filePath, args.content as string, 'utf-8')
      return { success: true, path: filePath }
    }
  },

  {
    name: 'edit_file',
    description: 'Find old_text in a file and replace it with new_text. Returns an error if old_text is not found.',
    parameters: {
      type: 'OBJECT',
      properties: {
        path: { type: 'STRING', description: 'Absolute or relative path to the file to edit' },
        old_text: { type: 'STRING', description: 'The exact text to find in the file' },
        new_text: { type: 'STRING', description: 'The text to replace old_text with' }
      },
      required: ['path', 'old_text', 'new_text']
    },
    destructive: false,
    execute: async (args) => {
      const filePath = path.resolve(args.path as string)
      const oldText = args.old_text as string
      const newText = args.new_text as string
      const content = await fs.readFile(filePath, 'utf-8')

      if (!content.includes(oldText)) {
        return { error: 'old_text not found in file' }
      }

      const updated = content.replace(oldText, newText)
      await fs.writeFile(filePath, updated, 'utf-8')
      return { success: true, path: filePath }
    }
  },

  {
    name: 'delete_file',
    description: 'Delete a file at the given path',
    parameters: {
      type: 'OBJECT',
      properties: {
        path: { type: 'STRING', description: 'Absolute or relative path to the file to delete' }
      },
      required: ['path']
    },
    destructive: true,
    execute: async (args) => {
      const filePath = path.resolve(args.path as string)
      await fs.unlink(filePath)
      return { success: true, path: filePath }
    }
  },

  {
    name: 'move_file',
    description: 'Move or rename a file from one path to another',
    parameters: {
      type: 'OBJECT',
      properties: {
        from: { type: 'STRING', description: 'Source path of the file to move' },
        to: { type: 'STRING', description: 'Destination path for the file' }
      },
      required: ['from', 'to']
    },
    destructive: false,
    execute: async (args) => {
      const fromPath = path.resolve(args.from as string)
      const toPath = path.resolve(args.to as string)
      await fs.mkdir(path.dirname(toPath), { recursive: true })
      await fs.rename(fromPath, toPath)
      return { success: true, from: fromPath, to: toPath }
    }
  },

  {
    name: 'copy_file',
    description: 'Copy a file from one path to another',
    parameters: {
      type: 'OBJECT',
      properties: {
        from: { type: 'STRING', description: 'Source path of the file to copy' },
        to: { type: 'STRING', description: 'Destination path for the copy' }
      },
      required: ['from', 'to']
    },
    destructive: false,
    execute: async (args) => {
      const fromPath = path.resolve(args.from as string)
      const toPath = path.resolve(args.to as string)
      await fs.mkdir(path.dirname(toPath), { recursive: true })
      await fs.copyFile(fromPath, toPath)
      return { success: true, from: fromPath, to: toPath }
    }
  },

  {
    name: 'list_directory',
    description: 'List the contents of a directory, returning name and type for each entry',
    parameters: {
      type: 'OBJECT',
      properties: {
        path: { type: 'STRING', description: 'Absolute or relative path to the directory to list' }
      },
      required: ['path']
    },
    destructive: false,
    execute: async (args) => {
      const dirPath = path.resolve(args.path as string)
      const dirents = await fs.readdir(dirPath, { withFileTypes: true })
      const entries = dirents.map((d) => ({
        name: d.name,
        type: d.isDirectory() ? 'directory' : d.isSymbolicLink() ? 'symlink' : 'file'
      }))
      return { entries }
    }
  },

  {
    name: 'search_files',
    description: 'Search for files matching a glob pattern in a directory. Returns up to 200 results.',
    parameters: {
      type: 'OBJECT',
      properties: {
        pattern: { type: 'STRING', description: 'Glob pattern to match files (e.g. "**/*.ts")' },
        directory: { type: 'STRING', description: 'Directory to search in' }
      },
      required: ['pattern', 'directory']
    },
    destructive: false,
    execute: async (args) => {
      const pattern = args.pattern as string
      const directory = path.resolve(args.directory as string)
      const { glob } = await import('glob')
      const files = await glob(pattern, { cwd: directory, nodir: true, absolute: true })
      return { files: files.slice(0, 200) }
    }
  },

  {
    name: 'search_content',
    description:
      'Search file contents using ripgrep (rg). Returns matching lines with file paths, limited to 50 results.',
    parameters: {
      type: 'OBJECT',
      properties: {
        pattern: { type: 'STRING', description: 'Search pattern (regex supported)' },
        directory: { type: 'STRING', description: 'Directory to search in' },
        file_glob: {
          type: 'STRING',
          description: 'Optional glob to filter files (e.g. "*.ts")'
        }
      },
      required: ['pattern', 'directory']
    },
    destructive: false,
    execute: async (args) => {
      const pattern = args.pattern as string
      const directory = path.resolve(args.directory as string)
      const fileGlob = args.file_glob as string | undefined

      const globArgs = fileGlob ? ` --glob '${fileGlob}'` : ''
      const cmd = `rg --max-count 50 --line-number --no-heading -- '${pattern.replace(/'/g, "'\\''")}'${globArgs} '${directory.replace(/'/g, "'\\''")}'`

      try {
        const output = execSync(cmd, { encoding: 'utf-8', maxBuffer: 1024 * 1024, timeout: 10000 })
        const lines = output.trim().split('\n').filter(Boolean)
        const matches = lines.slice(0, 50).map((line) => {
          const colonIdx = line.indexOf(':')
          const secondColon = line.indexOf(':', colonIdx + 1)
          if (colonIdx === -1 || secondColon === -1) return { file: '', line: 0, text: line }
          return {
            file: line.slice(0, colonIdx),
            line: parseInt(line.slice(colonIdx + 1, secondColon), 10),
            text: line.slice(secondColon + 1)
          }
        })
        return { matches }
      } catch (err: unknown) {
        const error = err as { status?: number; message?: string }
        // rg exits with code 1 when no matches found
        if (error.status === 1) {
          return { matches: [] }
        }
        return { error: error.message || 'ripgrep search failed' }
      }
    }
  },

  {
    name: 'file_info',
    description: 'Get metadata about a file: size, created, modified, isDirectory, and permissions',
    parameters: {
      type: 'OBJECT',
      properties: {
        path: { type: 'STRING', description: 'Absolute or relative path to the file' }
      },
      required: ['path']
    },
    destructive: false,
    execute: async (args) => {
      const filePath = path.resolve(args.path as string)
      const stats = await fs.stat(filePath)
      return {
        size: stats.size,
        created: stats.birthtime.toISOString(),
        modified: stats.mtime.toISOString(),
        isDirectory: stats.isDirectory(),
        permissions: '0' + (stats.mode & 0o777).toString(8)
      }
    }
  }
]
