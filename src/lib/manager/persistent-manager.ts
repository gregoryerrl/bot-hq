import { EventEmitter } from "events";
import { initializeBotHqStructure, BOT_HQ_ROOT, getManagerPrompt } from "@/lib/bot-hq";
import { getReInitPrompt } from "@/lib/bot-hq/templates";
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

You just started. Follow the "On Startup" section from your instructions above — ALL steps are MANDATORY:

1. Spawn an **Assistant Manager Bot** subagent (startup mode) — it handles ALL health checks, stuck task resets, diagram auditing, and git health auditing
2. Read the Assistant Manager Bot's report
3. For each workspace the report says has 0 diagrams: spawn a **Visualizer Bot** subagent
4. Wait for all Visualizer Bot subagents to complete
5. Wait for task commands

You are a pure orchestrator. Do NOT call health/context/diagram/git tools directly — the Assistant Manager Bot does that for you.`;
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

/**
 * Wait for the PTY session to become idle (no output for IDLE_DELAY ms),
 * then invoke the callback. Cleans up its listener afterward.
 */
function waitForIdle(callback: () => void, idleDelay = 4000, maxTimeout = 15000): void {
  const session = ptyManager.getSession(MANAGER_SESSION_ID);
  if (!session) {
    console.error("[Manager] No session found for idle wait");
    return;
  }

  let idleTimeout: NodeJS.Timeout | null = null;
  let fallbackTimeout: NodeJS.Timeout | null = null;
  let fired = false;
  let listener: ((data: string) => void) | null = null;

  const fire = () => {
    if (fired) return;
    fired = true;
    if (idleTimeout) clearTimeout(idleTimeout);
    if (fallbackTimeout) clearTimeout(fallbackTimeout);
    if (listener) {
      session.emitter.off("data", listener);
      listener = null;
    }
    callback();
  };

  const resetIdleTimer = () => {
    if (idleTimeout) clearTimeout(idleTimeout);
    idleTimeout = setTimeout(fire, idleDelay);
  };

  listener = () => {
    resetIdleTimer();
  };

  session.emitter.on("data", listener);
  resetIdleTimer();

  // Fallback timeout in case something goes wrong
  fallbackTimeout = setTimeout(() => {
    if (!fired) {
      console.log("[Manager] Fallback: firing after max timeout");
      fire();
    }
  }, maxTimeout);
}

class PersistentManager extends EventEmitter {
  private startupCommandSent = false;
  private outputListener: ((data: string) => void) | null = null;
  private isClearing = false;
  private pendingCommand: string | null = null;

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
    console.log("[Manager] Waiting for Claude Code to be ready (idle state)...");

    waitForIdle(() => {
      if (!this.startupCommandSent) {
        this.startupCommandSent = true;
        console.log("[Manager] Claude Code is idle (ready), sending startup command...");
        this.sendStartupCommand();
      }
    });
  }

  private async sendStartupCommand(): Promise<void> {
    console.log("[Manager] Building startup command with manager prompt...");
    const startupCommand = await buildStartupCommand();

    console.log("[Manager] Sending startup initialization command to Claude Code...");
    console.log("[Manager] Command length:", startupCommand.length);

    // Clear any existing input/autocomplete before writing
    ptyManager.write(MANAGER_SESSION_ID, "\u001b");  // Escape - dismiss suggestions
    await new Promise(resolve => setTimeout(resolve, 100));
    ptyManager.write(MANAGER_SESSION_ID, "\u0015");  // Ctrl+U - kill line
    await new Promise(resolve => setTimeout(resolve, 100));

    // Send text first (Claude Code treats this as a paste)
    ptyManager.write(MANAGER_SESSION_ID, startupCommand);

    // Wait for paste to be processed, then send Enter as separate write
    await new Promise(resolve => setTimeout(resolve, 500));
    ptyManager.write(MANAGER_SESSION_ID, "\u000d");

    console.log("[Manager] Startup command sent successfully");
  }

  // Send a command to the PTY-based manager session
  async sendCommand(command: string): Promise<void> {
    if (!ptyManager.hasManagerSession()) {
      console.error("[Manager] PTY session not initialized");
      // Try to start it
      await this.start();
    }

    // If currently clearing, queue the command for replay after re-init
    if (this.isClearing) {
      console.log("[Manager] Currently clearing, queuing command:", command.substring(0, 50));
      this.pendingCommand = command;
      return;
    }

    console.log("[Manager] Sending command to PTY:", command.substring(0, 100) + "...");

    // Wait for Claude Code to be idle (ready for input) before sending
    await new Promise<void>((resolve) => {
      waitForIdle(async () => {
        // Re-check isClearing inside the callback - a clear may have started
        // while we were waiting for idle
        if (this.isClearing) {
          console.log("[Manager] Clear started during idle wait, queuing command:", command.substring(0, 50));
          this.pendingCommand = command;
          resolve();
          return;
        }

        console.log("[Manager] Session idle, clearing input and writing command...");

        // Strategy: Prepend a random nonce to the command so it can NEVER
        // match Claude Code's autocomplete history. The manager prompt
        // is instructed to ignore the nonce prefix and extract the task ID.
        const nonce = Math.random().toString(36).slice(2, 8);
        const prefixedCommand = `${nonce} ${command}`;

        // Strategy: Type command character-by-character to avoid autocomplete
        // matching on pasted text. Claude Code autocomplete triggers on paste
        // but not on individual keystrokes that don't match history.

        // First dismiss any existing autocomplete and clear line
        ptyManager.write(MANAGER_SESSION_ID, "\u001b");  // Escape
        await new Promise(r => setTimeout(r, 100));
        ptyManager.write(MANAGER_SESSION_ID, "\u0015");  // Ctrl+U - kill line
        await new Promise(r => setTimeout(r, 100));

        // Type the command character by character with small delays
        // The nonce prefix + simplified format (TASK {id}) prevents autocomplete
        for (const char of prefixedCommand) {
          ptyManager.write(MANAGER_SESSION_ID, char);
          await new Promise(r => setTimeout(r, 20));
        }

        // Wait for typing to settle, then send Enter
        // Use 500ms gap to ensure Escape (if any) isn't combined with Enter as Alt+Enter
        await new Promise(r => setTimeout(r, 500));
        ptyManager.write(MANAGER_SESSION_ID, "\u000d");
        console.log("[Manager] Command submitted char-by-char with nonce:", nonce);
        resolve();
      }, 2000, 30000); // 2s idle threshold, 30s max timeout
    });
  }

  /**
   * Full restart: kill the PTY, spawn a fresh one, send the full startup prompt.
   * Used by the "Run Startup" button.
   * Ralph Wiggum style: no /clear, no autocomplete, no slash command picker.
   */
  async restartWithFullPrompt(): Promise<void> {
    this.isClearing = true;
    console.log("[Manager] Full restart: killing PTY and spawning fresh...");

    // Get scope path before killing
    let scopePath: string;
    try {
      scopePath = await getScopePath();
    } catch {
      scopePath = process.env.HOME || "/tmp";
    }

    // Kill the old session
    if (ptyManager.hasManagerSession()) {
      ptyManager.killSession(MANAGER_SESSION_ID);
    }

    // Spawn a fresh Claude Code session
    ptyManager.ensureManagerSession(scopePath);
    this.startupCommandSent = false;

    // Wait for Claude Code to fully boot (fixed delay — simple and reliable)
    console.log("[Manager] Waiting 8s for Claude Code to boot...");
    await new Promise(r => setTimeout(r, 8000));

    console.log("[Manager] Sending full startup prompt...");
    this.startupCommandSent = true;
    const startupCommand = await buildStartupCommand();

    // Clear input before pasting
    ptyManager.write(MANAGER_SESSION_ID, "\u001b");  // Escape
    await new Promise(r => setTimeout(r, 100));
    ptyManager.write(MANAGER_SESSION_ID, "\u0015");  // Ctrl+U
    await new Promise(r => setTimeout(r, 100));

    // Paste the prompt
    ptyManager.write(MANAGER_SESSION_ID, startupCommand);

    // Wait then Enter
    await new Promise(r => setTimeout(r, 500));
    ptyManager.write(MANAGER_SESSION_ID, "\u000d");
    console.log("[Manager] Full startup prompt sent successfully");

    this.isClearing = false;
    this.pendingCommand = null;
  }

  /**
   * Self-clear: kill the PTY, spawn a fresh one, send the re-init prompt.
   * No /clear slash command needed — just nuke it and start over.
   */
  async selfClear(): Promise<void> {
    this.isClearing = true;
    console.log("[Manager] Self-clear: killing PTY and spawning fresh...");

    // Get scope path before killing
    let scopePath: string;
    try {
      scopePath = await getScopePath();
    } catch {
      scopePath = process.env.HOME || "/tmp";
    }

    // Kill the old session
    if (ptyManager.hasManagerSession()) {
      ptyManager.killSession(MANAGER_SESSION_ID);
    }

    // Spawn a fresh Claude Code session
    ptyManager.ensureManagerSession(scopePath);
    this.startupCommandSent = false;

    // Wait for Claude Code to fully boot (fixed delay — simple and reliable)
    console.log("[Manager] Waiting 8s for Claude Code to boot...");
    await new Promise(r => setTimeout(r, 8000));

    console.log("[Manager] Sending re-init prompt...");
    this.startupCommandSent = true;
    const reInitPrompt = getReInitPrompt();

    // Clear input before pasting
    ptyManager.write(MANAGER_SESSION_ID, "\u001b");  // Escape
    await new Promise(r => setTimeout(r, 100));
    ptyManager.write(MANAGER_SESSION_ID, "\u0015");  // Ctrl+U
    await new Promise(r => setTimeout(r, 100));

    // Paste the prompt
    ptyManager.write(MANAGER_SESSION_ID, reInitPrompt);

    // Wait then Enter
    await new Promise(r => setTimeout(r, 500));
    ptyManager.write(MANAGER_SESSION_ID, "\u000d");
    console.log("[Manager] Re-init prompt sent successfully");

    this.isClearing = false;

    // Replay any command that was queued during the clear
    if (this.pendingCommand) {
      const cmd = this.pendingCommand;
      this.pendingCommand = null;
      console.log("[Manager] Replaying queued command after clear:", cmd.substring(0, 50));
      // Wait for the re-init prompt to be fully processed before sending
      await new Promise(r => setTimeout(r, 3000));
      await this.sendCommand(cmd);
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
  console.log("[Manager] sendManagerCommand called with:", command.substring(0, 50) + "...");
  const manager = getManager();
  manager.sendCommand(command);
}

export async function clearManager(): Promise<void> {
  const manager = getManager();
  await manager.selfClear();
}

export async function restartManager(): Promise<void> {
  const manager = getManager();
  await manager.restartWithFullPrompt();
}

export function getManagerStatus(): { running: boolean; sessionId: string | null } {
  const manager = getManager();
  return manager.getStatus();
}

export function getManagerSessionId(): string | null {
  const manager = getManager();
  return manager.getSessionId();
}
