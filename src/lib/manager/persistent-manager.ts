import { EventEmitter } from "events";
import { initializeBotHqStructure, BOT_HQ_ROOT } from "@/lib/bot-hq";
import { existsSync, writeFileSync, unlinkSync } from "fs";
import path from "path";
import { ptyManager, MANAGER_SESSION_ID } from "@/lib/pty-manager";
import { getScopePath } from "@/lib/settings";

// Use a file-based flag to persist state across Next.js workers
const STATUS_FILE = path.join(BOT_HQ_ROOT, ".manager-status");

function isManagerRunning(): boolean {
  // Check both file flag and actual PTY session
  return existsSync(STATUS_FILE) || ptyManager.hasManagerSession();
}

function setManagerRunning(running: boolean): void {
  if (running) {
    writeFileSync(STATUS_FILE, Date.now().toString());
  } else if (existsSync(STATUS_FILE)) {
    unlinkSync(STATUS_FILE);
  }
}

class PersistentManager extends EventEmitter {
  async start(): Promise<void> {
    if (isManagerRunning()) {
      console.log("[Manager] Already initialized");
      return;
    }

    // Initialize .bot-hq structure
    await initializeBotHqStructure();

    console.log("[Manager] Starting persistent PTY session...");

    // Get scope path for the manager session working directory
    let scopePath: string;
    try {
      scopePath = await getScopePath();
    } catch {
      scopePath = process.env.HOME || "/tmp";
    }

    // Ensure the PTY-based manager session exists
    ptyManager.ensureManagerSession(scopePath);

    setManagerRunning(true);
    console.log("[Manager] PTY session started");
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
    // The user will see this in the terminal and interact with it
    const success = ptyManager.write(MANAGER_SESSION_ID, command + "\n");

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
