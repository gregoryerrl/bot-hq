import { spawn, ChildProcess } from "child_process";
import { EventEmitter } from "events";
import { getManagerPrompt, initializeBotHqStructure, BOT_HQ_ROOT } from "@/lib/bot-hq";

class PersistentManager extends EventEmitter {
  private process: ChildProcess | null = null;
  private isRunning = false;
  private outputBuffer = "";

  async start(): Promise<void> {
    if (this.isRunning) {
      console.log("[Manager] Already running");
      return;
    }

    // Initialize .bot-hq structure
    await initializeBotHqStructure();

    // Get manager prompt
    const managerPrompt = await getManagerPrompt();

    console.log("[Manager] Starting persistent session...");

    this.process = spawn("claude", [
      "--dangerously-skip-permissions",
      "-p",
      "--output-format", "stream-json",
      "--mcp-config", "/Users/gregoryerrl/Projects/bot-hq/.mcp.json",
    ], {
      cwd: BOT_HQ_ROOT,
      env: { ...process.env },
    });

    this.isRunning = true;

    // Send startup prompt
    this.process.stdin?.write(managerPrompt);
    this.process.stdin?.write("\n\nPerform your startup tasks now.\n");
    // Don't end stdin - keep it open for commands

    this.process.stdout?.on("data", (data: Buffer) => {
      const text = data.toString();
      this.outputBuffer += text;
      this.emit("output", text);

      // Parse JSON lines
      const lines = this.outputBuffer.split("\n");
      this.outputBuffer = lines.pop() || "";

      for (const line of lines) {
        if (!line.trim()) continue;
        try {
          const output = JSON.parse(line);
          this.emit("json", output);

          // Extract text for display
          if (output.type === "assistant" && output.message?.content) {
            for (const block of output.message.content) {
              if (block.type === "text") {
                this.emit("text", block.text);
              }
            }
          }
        } catch {
          // Non-JSON output
          this.emit("text", line);
        }
      }
    });

    this.process.stderr?.on("data", (data: Buffer) => {
      const text = data.toString();
      console.error("[Manager stderr]", text);
      this.emit("stderr", text);
    });

    this.process.on("error", (err) => {
      console.error("[Manager] Process error:", err);
      this.isRunning = false;
      this.emit("error", err);
    });

    this.process.on("exit", (code) => {
      console.log("[Manager] Process exited with code:", code);
      this.isRunning = false;
      this.emit("exit", code);
    });
  }

  sendCommand(command: string): void {
    if (!this.process || !this.isRunning) {
      console.error("[Manager] Cannot send command - not running");
      return;
    }

    this.process.stdin?.write(command + "\n");
  }

  stop(): void {
    if (this.process) {
      this.process.kill("SIGTERM");
      this.isRunning = false;
    }
  }

  getStatus(): { running: boolean; pid: number | null } {
    return {
      running: this.isRunning,
      pid: this.process?.pid || null,
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
  if (!managerInstance) {
    return { running: false, pid: null };
  }
  return managerInstance.getStatus();
}
