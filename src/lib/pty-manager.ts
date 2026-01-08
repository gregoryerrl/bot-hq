import * as pty from "node-pty";
import os from "os";
import { EventEmitter } from "events";

interface PtySession {
  id: string;
  pty: pty.IPty;
  emitter: EventEmitter;
  createdAt: Date;
  lastActivityAt: Date;
}

class PtyManager {
  private sessions: Map<string, PtySession> = new Map();
  private cleanupInterval: NodeJS.Timeout | null = null;

  constructor() {
    // Clean up stale sessions every 5 minutes
    this.cleanupInterval = setInterval(() => this.cleanupStaleSessions(), 5 * 60 * 1000);
  }

  createSession(cwd: string): string {
    const id = crypto.randomUUID();
    const home = os.homedir();

    // Get shell - prefer zsh on macOS
    const shell = process.env.SHELL || "/bin/zsh";

    console.log("[PtyManager] Creating session with shell:", shell);
    console.log("[PtyManager] CWD:", cwd);

    const emitter = new EventEmitter();
    emitter.setMaxListeners(20);

    // Spawn Claude Code via login shell for proper PATH resolution
    const ptyProcess = pty.spawn(shell, ["-l", "-c", "claude"], {
      name: "xterm-256color",
      cols: 120,
      rows: 30,
      cwd,
      env: {
        ...process.env,
        HOME: home,
        TERM: "xterm-256color",
        COLORTERM: "truecolor",
      } as { [key: string]: string },
    });

    ptyProcess.onData((data) => {
      emitter.emit("data", data);
      const session = this.sessions.get(id);
      if (session) {
        session.lastActivityAt = new Date();
      }
    });

    ptyProcess.onExit(({ exitCode }) => {
      emitter.emit("exit", exitCode);
      this.sessions.delete(id);
    });

    this.sessions.set(id, {
      id,
      pty: ptyProcess,
      emitter,
      createdAt: new Date(),
      lastActivityAt: new Date(),
    });

    return id;
  }

  getSession(id: string): PtySession | undefined {
    return this.sessions.get(id);
  }

  write(id: string, data: string): boolean {
    const session = this.sessions.get(id);
    if (session) {
      session.pty.write(data);
      session.lastActivityAt = new Date();
      return true;
    }
    return false;
  }

  resize(id: string, cols: number, rows: number): boolean {
    const session = this.sessions.get(id);
    if (session) {
      session.pty.resize(cols, rows);
      return true;
    }
    return false;
  }

  killSession(id: string): boolean {
    const session = this.sessions.get(id);
    if (session) {
      session.pty.kill();
      session.emitter.removeAllListeners();
      this.sessions.delete(id);
      return true;
    }
    return false;
  }

  listSessions(): Array<{ id: string; createdAt: Date; lastActivityAt: Date }> {
    return Array.from(this.sessions.values()).map((s) => ({
      id: s.id,
      createdAt: s.createdAt,
      lastActivityAt: s.lastActivityAt,
    }));
  }

  private cleanupStaleSessions() {
    const now = new Date();
    const maxAge = 30 * 60 * 1000; // 30 minutes

    for (const [id, session] of this.sessions) {
      if (now.getTime() - session.lastActivityAt.getTime() > maxAge) {
        console.log(`[PtyManager] Cleaning up stale session: ${id}`);
        this.killSession(id);
      }
    }
  }

  destroy() {
    if (this.cleanupInterval) {
      clearInterval(this.cleanupInterval);
    }
    for (const id of this.sessions.keys()) {
      this.killSession(id);
    }
  }
}

// Singleton instance
const globalForPty = globalThis as unknown as { ptyManager: PtyManager };
export const ptyManager = globalForPty.ptyManager || new PtyManager();
if (process.env.NODE_ENV !== "production") {
  globalForPty.ptyManager = ptyManager;
}
