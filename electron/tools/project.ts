import { ToolDefinition } from './types'
import { getDb, schema } from '../memory/db'
import { scanProject } from '../memory/project-scanner'
import { eq, desc } from 'drizzle-orm'
import { v4 as uuid } from 'uuid'

export const projectTools: ToolDefinition[] = [
  {
    name: 'focus_project',
    description:
      'Set the active project by scanning a directory. Indexes the file tree, reads key config files, and stores project info. If a project with this path already exists it will be updated; otherwise a new record is created.',
    parameters: {
      type: 'OBJECT',
      properties: {
        path: { type: 'STRING', description: 'Absolute path to the project directory' }
      },
      required: ['path']
    },
    destructive: false,
    execute: async (args) => {
      const db = getDb()
      const projectPath = args.path as string
      const now = new Date().toISOString()

      const scan = await scanProject(projectPath)

      const fileCount = scan.tree.split('\n').filter(Boolean).length
      const keyFilesJson = JSON.stringify(scan.keyFiles)

      // Check if project already exists by path
      const existing = await db
        .select()
        .from(schema.projects)
        .where(eq(schema.projects.path, projectPath))
        .limit(1)

      if (existing.length > 0) {
        // Update existing project
        await db
          .update(schema.projects)
          .set({
            name: scan.name,
            fileTreeSnapshot: scan.tree,
            keyFiles: keyFilesJson,
            lastFocusedAt: now
          })
          .where(eq(schema.projects.path, projectPath))
      } else {
        // Insert new project
        await db.insert(schema.projects).values({
          id: uuid(),
          name: scan.name,
          path: projectPath,
          fileTreeSnapshot: scan.tree,
          keyFiles: keyFilesJson,
          lastFocusedAt: now,
          createdAt: now
        })
      }

      return {
        success: true,
        name: scan.name,
        path: projectPath,
        fileCount,
        keyFiles: Object.keys(scan.keyFiles)
      }
    }
  },

  {
    name: 'unfocus',
    description:
      'Clear project focus. Removes the lastFocusedAt timestamp from all projects so none is considered active.',
    parameters: {
      type: 'OBJECT',
      properties: {},
      required: []
    },
    destructive: false,
    execute: async () => {
      const db = getDb()

      await db
        .update(schema.projects)
        .set({ lastFocusedAt: null })

      return { success: true, message: 'Project focus cleared' }
    }
  },

  {
    name: 'project_status',
    description:
      'Show information about the currently focused project. Returns the most recently focused project or indicates that no project is focused.',
    parameters: {
      type: 'OBJECT',
      properties: {},
      required: []
    },
    destructive: false,
    execute: async () => {
      const db = getDb()

      const results = await db
        .select()
        .from(schema.projects)
        .orderBy(desc(schema.projects.lastFocusedAt))
        .limit(1)

      if (results.length === 0 || !results[0].lastFocusedAt) {
        return { focused: false }
      }

      const project = results[0]
      return {
        focused: true,
        name: project.name,
        path: project.path,
        description: project.description,
        lastFocusedAt: project.lastFocusedAt,
        fileTree: project.fileTreeSnapshot,
        keyFiles: project.keyFiles ? Object.keys(JSON.parse(project.keyFiles)) : [],
        conventions: project.conventions
      }
    }
  }
]
