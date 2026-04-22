import { getDb, schema } from './db'
import { desc } from 'drizzle-orm'

const BASE_SYSTEM_PROMPT = `You are Bot-HQ, a voice-controlled computer agent running on macOS. You help the user by executing tools on their machine.

You can:
- Read, write, edit, search, and manage files
- Run shell commands and control system processes
- Manage git repositories
- Take screenshots and understand what's on screen
- Open apps and URLs
- Remember facts and preferences for future conversations
- Focus on specific projects for context-aware assistance
- Start and manage Claude Code sessions for complex coding tasks

Guidelines:
- Be concise in voice responses — the user is listening, not reading
- For destructive actions (delete, kill, force push), always explain what you're about to do
- When focused on a project, scope file and git operations to that project by default
- Use the remember tool to store important facts the user tells you
- Use the recall tool to check memory before asking the user something they may have told you before
`

export async function buildSystemInstruction(): Promise<string> {
  const db = getDb()
  const parts: string[] = [BASE_SYSTEM_PROMPT]

  // Add long-term memories (most recently accessed first)
  const memories = await db
    .select()
    .from(schema.memories)
    .orderBy(desc(schema.memories.lastAccessedAt))
    .limit(30)

  if (memories.length > 0) {
    parts.push('\n## Your Memories')
    for (const m of memories) {
      parts.push(`- [${m.category}] ${m.content}`)
    }
  }

  // Add focused project context
  const focusedProject = await db
    .select()
    .from(schema.projects)
    .orderBy(desc(schema.projects.lastFocusedAt))
    .limit(1)

  if (focusedProject.length > 0 && focusedProject[0].lastFocusedAt) {
    const p = focusedProject[0]
    parts.push(`\n## Currently Focused Project: ${p.name}`)
    parts.push(`Path: ${p.path}`)
    if (p.description) parts.push(`Description: ${p.description}`)
    if (p.conventions) parts.push(`Conventions: ${p.conventions}`)
    if (p.keyFiles) {
      try {
        const keyFiles = JSON.parse(p.keyFiles)
        parts.push('\nKey files:')
        for (const [name, content] of Object.entries(keyFiles)) {
          parts.push(`\n### ${name}\n\`\`\`\n${(content as string).slice(0, 2000)}\n\`\`\``)
        }
      } catch {
        // Ignore malformed keyFiles JSON
      }
    }
  }

  // Add last conversation summary
  const lastConvo = await db
    .select()
    .from(schema.conversations)
    .orderBy(desc(schema.conversations.startedAt))
    .limit(1)

  if (lastConvo.length > 0 && lastConvo[0].summary) {
    parts.push(`\n## Last Conversation Summary\n${lastConvo[0].summary}`)
  }

  return parts.join('\n')
}
