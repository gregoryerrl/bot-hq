import { TmuxClient } from './client'
import { discoverClaudeSessions } from './discovery'
import { getDb, schema } from '../memory/db'
import { eq } from 'drizzle-orm'
import { v4 as uuid } from 'uuid'

export class ClaudeSessionManager {
  private tmux: TmuxClient

  constructor() {
    this.tmux = new TmuxClient()
  }

  isAvailable(): boolean {
    return this.tmux.isAvailable()
  }

  discover() {
    const found = discoverClaudeSessions(this.tmux)
    const db = getDb()
    const results = []

    for (const session of found) {
      const existing = db
        .select()
        .from(schema.claudeSessions)
        .where(eq(schema.claudeSessions.tmuxTarget, session.tmuxTarget))
        .all()

      if (existing.length > 0) {
        db.update(schema.claudeSessions)
          .set({
            pid: session.pid,
            status: 'running',
            lastCheckedAt: new Date().toISOString()
          })
          .where(eq(schema.claudeSessions.id, existing[0].id))
          .run()
        results.push({ ...existing[0], pid: session.pid, status: 'running' })
      } else {
        const id = uuid()
        const record = {
          id,
          projectPath: session.cwd,
          pid: session.pid,
          tmuxTarget: session.tmuxTarget,
          mode: 'attached' as const,
          status: 'running' as const,
          lastCheckedAt: new Date().toISOString(),
          startedAt: new Date().toISOString()
        }
        db.insert(schema.claudeSessions).values(record).run()
        results.push(record)
      }
    }

    return results
  }

  startSession(projectPath: string) {
    const target = this.tmux.newWindow('bot-hq', 'claude --dangerously-skip-permissions', projectPath)
    const db = getDb()
    const id = uuid()

    db.insert(schema.claudeSessions)
      .values({
        id,
        projectPath,
        tmuxTarget: target,
        mode: 'managed',
        status: 'running',
        startedAt: new Date().toISOString()
      })
      .run()

    return { id, tmuxTarget: target }
  }

  sendMessage(sessionId: string, message: string) {
    const db = getDb()
    const sessions = db
      .select()
      .from(schema.claudeSessions)
      .where(eq(schema.claudeSessions.id, sessionId))
      .all()

    if (!sessions.length || !sessions[0].tmuxTarget) {
      throw new Error('Session not found')
    }

    const prefixedMessage = `[BOT-HQ-BRIDGE v1] ${message}`
    this.tmux.sendKeys(sessions[0].tmuxTarget, prefixedMessage)

    const output = this.tmux.capturePane(sessions[0].tmuxTarget, 30)

    db.update(schema.claudeSessions)
      .set({
        lastOutput: output,
        lastCheckedAt: new Date().toISOString()
      })
      .where(eq(schema.claudeSessions.id, sessionId))
      .run()

    return output
  }

  readOutput(sessionId: string) {
    const db = getDb()
    const sessions = db
      .select()
      .from(schema.claudeSessions)
      .where(eq(schema.claudeSessions.id, sessionId))
      .all()

    if (!sessions.length || !sessions[0].tmuxTarget) {
      throw new Error('Session not found')
    }

    const output = this.tmux.capturePane(sessions[0].tmuxTarget, 50)

    db.update(schema.claudeSessions)
      .set({
        lastOutput: output,
        lastCheckedAt: new Date().toISOString()
      })
      .where(eq(schema.claudeSessions.id, sessionId))
      .run()

    return output
  }

  stopSession(sessionId: string) {
    const db = getDb()
    const sessions = db
      .select()
      .from(schema.claudeSessions)
      .where(eq(schema.claudeSessions.id, sessionId))
      .all()

    if (!sessions.length || !sessions[0].tmuxTarget) {
      throw new Error('Session not found')
    }

    this.tmux.killPane(sessions[0].tmuxTarget)

    db.update(schema.claudeSessions)
      .set({
        status: 'stopped',
        endedAt: new Date().toISOString()
      })
      .where(eq(schema.claudeSessions.id, sessionId))
      .run()
  }
}
