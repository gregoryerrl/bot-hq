import { EventEmitter } from "events";
import { initializeBotHqStructure, BOT_HQ_ROOT } from "@/lib/bot-hq";
import { existsSync, writeFileSync, unlinkSync } from "fs";
import path from "path";
import { ptyManager, MANAGER_SESSION_ID } from "@/lib/pty-manager";
import { getScopePath } from "@/lib/settings";

// Use a file-based flag to persist state across Next.js workers
const STATUS_FILE = path.join(BOT_HQ_ROOT, ".manager-status");

// Startup command for Claude Code to run initialization tasks
const STARTUP_COMMAND = `You are the bot-hq manager. Perform startup initialization:

1. Use status_overview to check system health
2. Use task_list to find any tasks stuck in "in_progress" state
3. For each stuck task, use task_update to reset its state to "queued" (these are orphaned from previous session)
4. Report what you found and any actions taken

Then wait for further instructions.`;

function isManagerRunning(): boolean {
  const hasStatusFile = existsSync(STATUS_FILE);
  const hasPtySession = ptyManager.hasManagerSession();

  // Clean up stale status file if PTY session doesn't exist
  if (hasStatusFile && !hasPtySession) {
    console.log("[Manager] Cleaning up stale status file (no PTY session)");
    try {
      unlinkSync(STATUS_FILE);
    } catch {
      // Ignore errors
    }
    return false;
  }

  return hasStatusFile || hasPtySession;
}

function setManagerRunning(running: boolean): void {
  if (running) {
    writeFileSync(STATUS_FILE, Date.now().toString());
  } else if (existsSync(STATUS_FILE)) {
    unlinkSync(STATUS_FILE);
  }
}

class PersistentManager extends EventEmitter {
  private startupCommandSent = false;
  private outputListener: ((data: string) => void) | null = null;

  async start(): Promise<void> {
    if (isManagerRunning()) {
      console.log("[Manager] Already initialized");
      return;
    }

    // Initialize .bot-hq structure first
    await initializeBotHqStructure();

    console.log("[Manager] Starting Claude Code PTY session...");

    // Get scope path for the manager session working directory
    let scopePath: string;
    try {
      scopePath = await getScopePath();
    } catch {
      scopePath = process.env.HOME || "/tmp";
    }

    // Start the PTY-based manager session (Claude Code)
    ptyManager.ensureManagerSession(scopePath);
    setManagerRunning(true);
    console.log("[Manager] Claude Code PTY session started");

    // Wait for Claude Code to be ready before sending startup command
    if (!this.startupCommandSent) {
      this.waitForReadyAndSendCommand();
    }
  }

  private waitForReadyAndSendCommand(): void {
    const session = ptyManager.getSession(MANAGER_SESSION_ID);
    if (!session) {
      console.error("[Manager] No session found for startup command");
      return;
    }

    console.log("[Manager] Waiting for Claude Code to be ready...");

    let buffer = "";

    this.outputListener = (data: string) => {
      buffer += data;

      // Claude Code shows ">" prompt or asks for input when ready
      // Look for patterns that indicate it's ready for input
      if (buffer.includes(">") || buffer.includes("What would you like to do?") || buffer.includes("How can I help")) {
        if (!this.startupCommandSent) {
          this.startupCommandSent = true;
          console.log("[Manager] Claude Code is ready, sending startup command...");

          // Remove listener
          if (this.outputListener) {
            session.emitter.off("data", this.outputListener);
            this.outputListener = null;
          }

          // Small delay to ensure prompt is fully rendered
          setTimeout(() => {
            this.sendStartupCommand();
          }, 500);
        }
      }
    };

    session.emitter.on("data", this.outputListener);

    // Fallback timeout in case we miss the ready signal
    setTimeout(() => {
      if (!this.startupCommandSent) {
        this.startupCommandSent = true;
        console.log("[Manager] Fallback: Sending startup command after timeout...");

        if (this.outputListener) {
          session.emitter.off("data", this.outputListener);
          this.outputListener = null;
        }

        this.sendStartupCommand();
      }
    }, 10000); // 10 second fallback
  }

  private sendStartupCommand(): void {
    console.log("[Manager] Sending startup initialization command to Claude Code...");
    const success = ptyManager.write(MANAGER_SESSION_ID, STARTUP_COMMAND + "\r");
    if (!success) {
      console.error("[Manager] Failed to send startup command");
    } else {
      console.log("[Manager] Startup command sent successfully");
    }
  }

  // Send a command to the PTY-based manager session
  async sendCommand(command: string): Promise<void> {
    if (!ptyManager.hasManagerSession()) {
      console.error("[Manager] PTY session not initialized");
      // Try to start it
      await this.start();
    }

    console.log("[Manager] Sending command to PTY:", command.substring(0, 100) + "...");

    // Write the command to the PTY session
    // Use \r (carriage return) to submit - this is what terminals expect
    const success = ptyManager.write(MANAGER_SESSION_ID, command + "\r");

    if (!success) {
      console.error("[Manager] Failed to write to PTY session");
    }
  }

  stop(): void {
    // Kill the PTY session if running
    if (ptyManager.hasManagerSession()) {
      ptyManager.killSession(MANAGER_SESSION_ID);
    }
    setManagerRunning(false);
  }

  getStatus(): { running: boolean; sessionId: string | null } {
    const hasSession = ptyManager.hasManagerSession();
    return {
      running: hasSession || isManagerRunning(),
      sessionId: hasSession ? MANAGER_SESSION_ID : null,
    };
  }

  // Get the manager session ID for connecting from the UI
  getSessionId(): string | null {
    return ptyManager.hasManagerSession() ? MANAGER_SESSION_ID : null;
  }
}

// Singleton instance
let managerInstance: PersistentManager | null = null;

export function getManager(): PersistentManager {
  if (!managerInstance) {
    managerInstance = new PersistentManager();
  }
  return managerInstance;
}

export async function startManager(): Promise<void> {
  const manager = getManager();
  await manager.start();
}

export function stopManager(): void {
  if (managerInstance) {
    managerInstance.stop();
  }
}

export function sendManagerCommand(command: string): void {
  const manager = getManager();
  manager.sendCommand(command);
}

export function getManagerStatus(): { running: boolean; sessionId: string | null } {
  const manager = getManager();
  return manager.getStatus();
}

export function getManagerSessionId(): string | null {
  const manager = getManager();
  return manager.getSessionId();
}
