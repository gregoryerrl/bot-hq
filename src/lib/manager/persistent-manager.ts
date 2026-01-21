import { spawn, ChildProcess } from "child_process";
import { EventEmitter } from "events";
import { getManagerPrompt, initializeBotHqStructure, BOT_HQ_ROOT } from "@/lib/bot-hq";
import { existsSync, writeFileSync, unlinkSync } from "fs";
import path from "path";

// Use a file-based flag to persist state across Next.js workers
const STATUS_FILE = path.join(BOT_HQ_ROOT, ".manager-status");

function isManagerRunning(): boolean {
  return existsSync(STATUS_FILE);
}

function setManagerRunning(running: boolean): void {
  if (running) {
    writeFileSync(STATUS_FILE, Date.now().toString());
  } else if (existsSync(STATUS_FILE)) {
    unlinkSync(STATUS_FILE);
  }
}

class PersistentManager extends EventEmitter {
  private commandQueue: string[] = [];
  private processing = false;

  async start(): Promise<void> {
    if (isManagerRunning()) {
      console.log("[Manager] Already initialized");
      return;
    }

    // Initialize .bot-hq structure
    await initializeBotHqStructure();

    console.log("[Manager] Starting persistent session...");
    setManagerRunning(true);
  }

  async processCommand(command: string): Promise<void> {
    const managerPrompt = await getManagerPrompt();
    const fullPrompt = `${managerPrompt}\n\n---\n\nUser Command:\n${command}`;

    return new Promise((resolve, reject) => {
      console.log("[Manager] Processing command:", command.substring(0, 100) + "...");

      const proc = spawn("claude", [
        "--dangerously-skip-permissions",
        "-p",
        "--output-format", "json",
        "--mcp-config", "/Users/gregoryerrl/Projects/bot-hq/.mcp.json",
      ], {
        cwd: BOT_HQ_ROOT,
        env: { ...process.env },
      });

      let stdout = "";
      let stderr = "";

      proc.stdout?.on("data", (data: Buffer) => {
        stdout += data.toString();
        this.emit("output", data.toString());
      });

      proc.stderr?.on("data", (data: Buffer) => {
        stderr += data.toString();
        console.error("[Manager stderr]", data.toString());
      });

      proc.on("error", (err) => {
        console.error("[Manager] Process error:", err);
        reject(err);
      });

      proc.on("exit", (code) => {
        console.log("[Manager] Command completed with exit code:", code);
        if (code === 0) {
          try {
            const result = JSON.parse(stdout);
            this.emit("result", result);
            resolve();
          } catch {
            this.emit("text", stdout);
            resolve();
          }
        } else {
          console.error("[Manager] Process failed:", stderr);
          reject(new Error(`Manager exited with code ${code}`));
        }
      });

      // Send the prompt
      proc.stdin?.write(fullPrompt);
      proc.stdin?.end();
    });
  }

  async sendCommand(command: string): Promise<void> {
    if (!isManagerRunning()) {
      console.error("[Manager] Not initialized");
      return;
    }

    this.commandQueue.push(command);

    if (!this.processing) {
      this.processing = true;
      while (this.commandQueue.length > 0) {
        const cmd = this.commandQueue.shift()!;
        try {
          await this.processCommand(cmd);
        } catch (error) {
          console.error("[Manager] Command failed:", error);
        }
      }
      this.processing = false;
    }
  }

  stop(): void {
    setManagerRunning(false);
  }

  getStatus(): { running: boolean; pid: number | null } {
    return {
      running: isManagerRunning(),
      pid: null, // No persistent process - spawns on demand
    };
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

export function getManagerStatus(): { running: boolean; pid: number | null } {
  return {
    running: isManagerRunning(),
    pid: null,
  };
}
