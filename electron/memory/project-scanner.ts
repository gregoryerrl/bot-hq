import { readdir, readFile } from 'fs/promises'
import { join, basename } from 'path'

const KEY_FILE_NAMES = [
  'package.json', 'README.md', 'CLAUDE.md', 'tsconfig.json',
  'Cargo.toml', 'go.mod', 'pyproject.toml', 'Makefile',
  '.env.example', 'docker-compose.yml', 'Dockerfile'
]

const IGNORE_DIRS = new Set([
  'node_modules', '.git', '.next', 'dist', 'out', 'build',
  '__pycache__', '.venv', 'vendor', 'target', '.turbo'
])

export async function scanProject(projectPath: string) {
  const tree = await buildFileTree(projectPath, 3)
  const keyFiles = await readKeyFiles(projectPath)
  const name = basename(projectPath)
  return { name, tree, keyFiles }
}

async function buildFileTree(dir: string, maxDepth: number, depth = 0): Promise<string> {
  if (depth >= maxDepth) return ''
  const entries = await readdir(dir, { withFileTypes: true })
  const lines: string[] = []
  const indent = '  '.repeat(depth)
  for (const entry of entries.sort((a, b) => a.name.localeCompare(b.name))) {
    if (IGNORE_DIRS.has(entry.name) || entry.name.startsWith('.')) continue
    if (entry.isDirectory()) {
      lines.push(`${indent}${entry.name}/`)
      lines.push(await buildFileTree(join(dir, entry.name), maxDepth, depth + 1))
    } else {
      lines.push(`${indent}${entry.name}`)
    }
  }
  return lines.filter(Boolean).join('\n')
}

async function readKeyFiles(dir: string): Promise<Record<string, string>> {
  const result: Record<string, string> = {}
  for (const name of KEY_FILE_NAMES) {
    try {
      const content = await readFile(join(dir, name), 'utf-8')
      result[name] = content.slice(0, 3000)
    } catch {
      // File doesn't exist or isn't readable — skip
    }
  }
  return result
}
