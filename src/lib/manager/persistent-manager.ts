import { EventEmitter } from "events";
import { initializeBotHqStructure, BOT_HQ_ROOT, getManagerPrompt } from "@/lib/bot-hq";
import { existsSync, writeFileSync, unlinkSync } from "fs";
import path from "path";
import { ptyManager, MANAGER_SESSION_ID } from "@/lib/pty-manager";
import { getScopePath } from "@/lib/settings";

// Use a file-based flag to persist state across Next.js workers
const STATUS_FILE = path.join(BOT_HQ_ROOT, ".manager-status");

// Build startup command with manager prompt
async function buildStartupCommand(): Promise<string> {
  const managerPrompt = await getManagerPrompt();

  return `${managerPrompt}

---

# STARTUP INITIALIZATION

You just started. Perform these initialization tasks:

1. Use status_overview to check system health
2. Use task_list to find any tasks stuck in "in_progress" state
3. For each stuck task, use task_update to reset its state to "queued" (these are orphaned from previous session)
4. Report what you found and any actions taken

Then wait for task commands. When you receive a task command, follow your instructions above.`;
}

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

    console.log("[Manager] Waiting for Claude Code to be ready (idle state)...");

    let idleTimeout: NodeJS.Timeout | null = null;
    const IDLE_DELAY = 2000; // Wait 2 seconds of no output = idle/ready

    const resetIdleTimer = () => {
      if (idleTimeout) {
        clearTimeout(idleTimeout);
      }
      idleTimeout = setTimeout(() => {
        if (!this.startupCommandSent) {
          this.startupCommandSent = true;
          console.log("[Manager] Claude Code is idle (ready), sending startup command...");

          // Remove listener
          if (this.outputListener) {
            session.emitter.off("data", this.outputListener);
            this.outputListener = null;
          }

          this.sendStartupCommand();
        }
      }, IDLE_DELAY);
    };

    this.outputListener = () => {
      // Each time we get output, reset the idle timer
      resetIdleTimer();
    };

    session.emitter.on("data", this.outputListener);

    // Start the idle timer immediately
    resetIdleTimer();

    // Fallback timeout in case something goes wrong
    setTimeout(() => {
      if (!this.startupCommandSent) {
        this.startupCommandSent = true;
        console.log("[Manager] Fallback: Sending startup command after max timeout...");

        if (idleTimeout) {
          clearTimeout(idleTimeout);
        }
        if (this.outputListener) {
          session.emitter.off("data", this.outputListener);
          this.outputListener = null;
        }

        this.sendStartupCommand();
      }
    }, 15000); // 15 second max fallback
  }

  private async sendStartupCommand(): Promise<void> {
    console.log("[Manager] Building startup command with manager prompt...");
    const startupCommand = await buildStartupCommand();

    console.log("[Manager] Sending startup initialization command to Claude Code...");

    // Use bracketed paste mode (like xterm.js does)
    const PASTE_START = "\x1b[200~";
    const PASTE_END = "\x1b[201~";
    const FOCUS_IN = "\x1b[I";

    // Send paste content
    ptyManager.write(MANAGER_SESSION_ID, PASTE_START + startupCommand + PASTE_END);

    // Wait for Claude Code to process the paste
    await new Promise(resolve => setTimeout(resolve, 500));

    // Send focus sequence then Enter (like browser does)
    ptyManager.write(MANAGER_SESSION_ID, FOCUS_IN);
    await new Promise(resolve => setTimeout(resolve, 50));
    const success = ptyManager.write(MANAGER_SESSION_ID, "\r");

    if (!success) {
      console.error("[Manager] Failed to send Enter key");
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

    // Use bracketed paste mode (like xterm.js does)
    const PASTE_START = "\x1b[200~";
    const PASTE_END = "\x1b[201~";
    const FOCUS_IN = "\x1b[I";

    // Send paste content
    ptyManager.write(MANAGER_SESSION_ID, PASTE_START + command + PASTE_END);

    // Wait for processing
    await new Promise(resolve => setTimeout(resolve, 200));

    // Send focus sequence then Enter
    ptyManager.write(MANAGER_SESSION_ID, FOCUS_IN);
    await new Promise(resolve => setTimeout(resolve, 50));
    ptyManager.write(MANAGER_SESSION_ID, "\r");
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
