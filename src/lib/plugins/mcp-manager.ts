// src/lib/plugins/mcp-manager.ts

import { spawn, ChildProcess } from "child_process";
import { join } from "path";
import { access } from "fs/promises";
import { LoadedPlugin } from "./types";
import { getPluginRegistry } from "./registry";

interface McpServer {
  plugin: LoadedPlugin;
  process: ChildProcess | null;
  status: "stopped" | "starting" | "running" | "error";
  errorMessage?: string;
  lastError?: Date;
  restartCount: number;
  pendingCalls: Map<string, {
    resolve: (value: unknown) => void;
    reject: (error: Error) => void;
    timeoutId: NodeJS.Timeout;
  }>;
  callId: number;
}

// Configuration
const MAX_RESTART_ATTEMPTS = 3;
const RESTART_DELAY_MS = 2000;
const CALL_TIMEOUT_MS = 30000;
const STARTUP_TIMEOUT_MS = 5000;

class McpManager {
  private servers: Map<string, McpServer> = new Map();

  async startServer(plugin: LoadedPlugin): Promise<void> {
    if (!plugin.manifest.mcp) {
      throw new Error(`Plugin ${plugin.name} has no MCP configuration`);
    }

    const existing = this.servers.get(plugin.name);
    if (existing?.status === "running") {
      return;
    }

    // Initialize or update server entry
    const server: McpServer = existing || {
      plugin,
      process: null,
      status: "starting",
      pendingCalls: new Map(),
      callId: 0,
      restartCount: 0,
    };
    server.status = "starting";
    server.errorMessage = undefined;
    this.servers.set(plugin.name, server);

    const entryPath = join(plugin.path, plugin.manifest.mcp.entry);

    // Verify entry file exists
    try {
      await access(entryPath);
    } catch {
      const error = `MCP entry file not found: ${entryPath}`;
      server.status = "error";
      server.errorMessage = error;
      server.lastError = new Date();
      throw new Error(error);
    }

    // Get credentials from registry to pass as env
    const registry = getPluginRegistry();
    let credentials: Record<string, string> = {};
    try {
      credentials = await registry.getCredentials(plugin.name);
    } catch (err) {
      console.warn(`[${plugin.name}] Failed to get credentials:`, err);
      // Continue without credentials - they may not be required
    }

    const env = {
      ...process.env,
      ...credentials,
    };

    // Spawn the MCP server process
    let child: ChildProcess;
    try {
      child = spawn("npx", ["tsx", entryPath], {
        cwd: plugin.path,
        env,
        stdio: ["pipe", "pipe", "pipe"],
      });
    } catch (err) {
      const error = `Failed to spawn MCP server: ${err instanceof Error ? err.message : "Unknown error"}`;
      server.status = "error";
      server.errorMessage = error;
      server.lastError = new Date();
      throw new Error(error);
    }

    server.process = child;

    let buffer = "";
    let stderrOutput = "";

    child.stdout?.on("data", (data) => {
      buffer += data.toString();

      // Process complete JSON-RPC messages
      const lines = buffer.split("\n");
      buffer = lines.pop() || "";

      for (const line of lines) {
        if (!line.trim()) continue;
        try {
          const message = JSON.parse(line);
          this.handleMessage(plugin.name, message);
        } catch {
          console.error(`[${plugin.name}] Failed to parse MCP message:`, line);
        }
      }
    });

    child.stderr?.on("data", (data) => {
      const output = data.toString();
      stderrOutput += output;
      console.error(`[${plugin.name}] MCP stderr:`, output);
    });

    child.on("exit", (code, signal) => {
      const exitReason = signal ? `signal ${signal}` : `code ${code}`;
      console.log(`[${plugin.name}] MCP server exited with ${exitReason}`);

      // Store error info
      if (code !== 0) {
        server.status = "error";
        server.errorMessage = stderrOutput.trim() || `Process exited with ${exitReason}`;
        server.lastError = new Date();
      } else {
        server.status = "stopped";
      }
      server.process = null;

      // Reject any pending calls with detailed error
      const exitError = new Error(
        `MCP server exited with ${exitReason}${stderrOutput ? `: ${stderrOutput.trim().slice(0, 200)}` : ""}`
      );
      for (const [, { reject, timeoutId }] of server.pendingCalls) {
        clearTimeout(timeoutId);
        reject(exitError);
      }
      server.pendingCalls.clear();

      // Auto-restart if it was running and crashed unexpectedly
      if (code !== 0 && server.restartCount < MAX_RESTART_ATTEMPTS) {
        this.scheduleRestart(plugin.name);
      }
    });

    child.on("error", (error) => {
      console.error(`[${plugin.name}] MCP server error:`, error);
      server.status = "error";
      server.errorMessage = error.message;
      server.lastError = new Date();
    });

    // Wait for server to be ready with timeout
    const startupSuccess = await this.waitForStartup(plugin.name, child);
    if (!startupSuccess) {
      const error = `MCP server failed to start within ${STARTUP_TIMEOUT_MS}ms`;
      server.status = "error";
      server.errorMessage = stderrOutput.trim() || error;
      server.lastError = new Date();
      this.stopServer(plugin.name);
      throw new Error(server.errorMessage);
    }

    server.status = "running";
    server.restartCount = 0; // Reset restart count on successful start
    console.log(`[${plugin.name}] MCP server started`);
  }

  private async waitForStartup(pluginName: string, child: ChildProcess): Promise<boolean> {
    return new Promise((resolve) => {
      const timeout = setTimeout(() => {
        resolve(false);
      }, STARTUP_TIMEOUT_MS);

      // Check if process exits early (failure)
      const onExit = () => {
        clearTimeout(timeout);
        resolve(false);
      };
      child.once("exit", onExit);

      // For now, just wait a bit for the process to stabilize
      // A more robust implementation would wait for an initialize response
      setTimeout(() => {
        clearTimeout(timeout);
        child.off("exit", onExit);
        // If process is still running, consider it started
        if (child.exitCode === null && !child.killed) {
          resolve(true);
        } else {
          resolve(false);
        }
      }, 1000);
    });
  }

  private scheduleRestart(pluginName: string): void {
    const server = this.servers.get(pluginName);
    if (!server) return;

    server.restartCount++;
    console.log(
      `[${pluginName}] Scheduling restart (attempt ${server.restartCount}/${MAX_RESTART_ATTEMPTS}) in ${RESTART_DELAY_MS}ms`
    );

    setTimeout(async () => {
      try {
        await this.startServer(server.plugin);
        console.log(`[${pluginName}] Auto-restart successful`);
      } catch (err) {
        console.error(`[${pluginName}] Auto-restart failed:`, err);
      }
    }, RESTART_DELAY_MS);
  }

  async stopServer(pluginName: string): Promise<void> {
    const server = this.servers.get(pluginName);
    if (!server?.process) return;

    // Clear any pending calls before stopping
    for (const [, { reject, timeoutId }] of server.pendingCalls) {
      clearTimeout(timeoutId);
      reject(new Error("MCP server stopped"));
    }
    server.pendingCalls.clear();

    server.process.kill("SIGTERM");
    server.status = "stopped";
    server.process = null;
  }

  async callTool(
    pluginName: string,
    tool: string,
    params: Record<string, unknown>
  ): Promise<unknown> {
    const server = this.servers.get(pluginName);

    if (!server) {
      // Try to start the server
      const plugin = getPluginRegistry().getPlugin(pluginName);
      if (!plugin) {
        throw new Error(`Plugin "${pluginName}" not found. Is the plugin installed?`);
      }
      await this.startServer(plugin);
      return this.callTool(pluginName, tool, params);
    }

    // Handle error state with informative message
    if (server.status === "error") {
      const errorDetails = server.errorMessage || "Unknown error";
      throw new Error(
        `MCP server for "${pluginName}" is in error state: ${errorDetails}. ` +
        `Restart the plugin or check the logs.`
      );
    }

    // Handle starting state - wait briefly then retry
    if (server.status === "starting") {
      await new Promise(resolve => setTimeout(resolve, 1000));
      return this.callTool(pluginName, tool, params);
    }

    if (server.status !== "running") {
      // Try to restart the server
      try {
        await this.startServer(server.plugin);
        return this.callTool(pluginName, tool, params);
      } catch (err) {
        throw new Error(
          `MCP server for "${pluginName}" is not running and failed to start: ${err instanceof Error ? err.message : "Unknown error"}`
        );
      }
    }

    // Verify process is still alive
    if (!server.process || server.process.killed) {
      server.status = "stopped";
      return this.callTool(pluginName, tool, params); // Retry will attempt restart
    }

    const id = String(++server.callId);

    const request = {
      jsonrpc: "2.0",
      id,
      method: "tools/call",
      params: {
        name: tool,
        arguments: params,
      },
    };

    return new Promise((resolve, reject) => {
      // Set up timeout
      const timeoutId = setTimeout(() => {
        if (server.pendingCalls.has(id)) {
          server.pendingCalls.delete(id);
          reject(new Error(
            `MCP call to "${tool}" timed out after ${CALL_TIMEOUT_MS}ms. ` +
            `The tool may be slow or the server unresponsive.`
          ));
        }
      }, CALL_TIMEOUT_MS);

      server.pendingCalls.set(id, { resolve, reject, timeoutId });

      try {
        const success = server.process?.stdin?.write(JSON.stringify(request) + "\n");
        if (!success) {
          clearTimeout(timeoutId);
          server.pendingCalls.delete(id);
          reject(new Error(`Failed to send request to MCP server for "${pluginName}"`));
        }
      } catch (err) {
        clearTimeout(timeoutId);
        server.pendingCalls.delete(id);
        reject(new Error(
          `Failed to communicate with MCP server: ${err instanceof Error ? err.message : "Unknown error"}`
        ));
      }
    });
  }

  private handleMessage(pluginName: string, message: { id?: string; result?: unknown; error?: { message: string; code?: number } }): void {
    const server = this.servers.get(pluginName);
    if (!server) return;

    if (message.id) {
      const pending = server.pendingCalls.get(message.id);
      if (pending) {
        clearTimeout(pending.timeoutId);
        server.pendingCalls.delete(message.id);
        if (message.error) {
          const errorCode = message.error.code ? ` (code: ${message.error.code})` : "";
          pending.reject(new Error(`MCP tool error${errorCode}: ${message.error.message}`));
        } else {
          pending.resolve(message.result);
        }
      }
    }
  }

  getServerStatus(pluginName: string): string {
    return this.servers.get(pluginName)?.status || "not_loaded";
  }

  getServerError(pluginName: string): { message: string; timestamp: Date } | null {
    const server = this.servers.get(pluginName);
    if (!server || !server.errorMessage) return null;
    return {
      message: server.errorMessage,
      timestamp: server.lastError || new Date(),
    };
  }

  getServerInfo(pluginName: string): {
    status: string;
    errorMessage?: string;
    restartCount: number;
    pendingCalls: number;
  } | null {
    const server = this.servers.get(pluginName);
    if (!server) return null;
    return {
      status: server.status,
      errorMessage: server.errorMessage,
      restartCount: server.restartCount,
      pendingCalls: server.pendingCalls.size,
    };
  }

  async restartServer(pluginName: string): Promise<void> {
    const server = this.servers.get(pluginName);
    if (!server) {
      throw new Error(`No server found for plugin "${pluginName}"`);
    }

    // Stop existing server
    await this.stopServer(pluginName);

    // Reset restart count for manual restart
    server.restartCount = 0;

    // Start fresh
    await this.startServer(server.plugin);
  }

  async stopAll(): Promise<void> {
    const stopPromises = Array.from(this.servers.keys()).map(name =>
      this.stopServer(name).catch(err =>
        console.error(`[${name}] Failed to stop server:`, err)
      )
    );
    await Promise.all(stopPromises);
  }
}

// Singleton instance
let managerInstance: McpManager | null = null;

export function getMcpManager(): McpManager {
  if (!managerInstance) {
    managerInstance = new McpManager();
  }
  return managerInstance;
}
