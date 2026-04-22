import { ToolDefinition } from './types'
import { getDb, schema } from '../memory/db'
import { eq, like, desc, sql } from 'drizzle-orm'
import { v4 as uuid } from 'uuid'

export const memoryTools: ToolDefinition[] = [
  {
    name: 'remember',
    description:
      'Store a fact in long-term memory. Use this to save preferences, decisions, facts about people, or project details that should be recalled later.',
    parameters: {
      type: 'OBJECT',
      properties: {
        content: { type: 'STRING', description: 'The fact or information to remember' },
        category: {
          type: 'STRING',
          description: 'Category of the memory',
          enum: ['preference', 'fact', 'decision', 'person', 'project']
        }
      },
      required: ['content', 'category']
    },
    destructive: false,
    execute: async (args) => {
      const db = getDb()
      const id = uuid()
      const now = new Date().toISOString()

      await db.insert(schema.memories).values({
        id,
        content: args.content as string,
        category: args.category as string,
        createdAt: now,
        lastAccessedAt: now,
        accessCount: 0
      })

      return { success: true, id, message: 'Memory stored successfully' }
    }
  },

  {
    name: 'recall',
    description:
      'Search long-term memory for previously stored facts. Returns up to 20 matching memories, ordered by most recently accessed.',
    parameters: {
      type: 'OBJECT',
      properties: {
        query: { type: 'STRING', description: 'Search term to match against memory content' },
        category: {
          type: 'STRING',
          description: 'Optional category filter',
          enum: ['preference', 'fact', 'decision', 'person', 'project']
        }
      },
      required: ['query']
    },
    destructive: false,
    execute: async (args) => {
      const db = getDb()
      const query = args.query as string
      const category = args.category as string | undefined

      const conditions = [like(schema.memories.content, `%${query}%`)]
      if (category) {
        conditions.push(eq(schema.memories.category, category))
      }

      const where =
        conditions.length === 1
          ? conditions[0]
          : sql`${conditions[0]} AND ${conditions[1]}`

      const results = await db
        .select()
        .from(schema.memories)
        .where(where)
        .orderBy(desc(schema.memories.lastAccessedAt))
        .limit(20)

      // Update access timestamps and counts for returned results
      const now = new Date().toISOString()
      for (const memory of results) {
        await db
          .update(schema.memories)
          .set({
            lastAccessedAt: now,
            accessCount: sql`${schema.memories.accessCount} + 1`
          })
          .where(eq(schema.memories.id, memory.id))
      }

      return { memories: results, count: results.length }
    }
  },

  {
    name: 'forget',
    description: 'Remove a specific memory by its ID. This permanently deletes the memory.',
    parameters: {
      type: 'OBJECT',
      properties: {
        id: { type: 'STRING', description: 'The ID of the memory to delete' }
      },
      required: ['id']
    },
    destructive: true,
    execute: async (args) => {
      const db = getDb()
      const id = args.id as string

      const existing = await db
        .select()
        .from(schema.memories)
        .where(eq(schema.memories.id, id))
        .limit(1)

      if (existing.length === 0) {
        return { success: false, error: 'Memory not found' }
      }

      await db.delete(schema.memories).where(eq(schema.memories.id, id))

      return { success: true, message: 'Memory deleted' }
    }
  }
]
