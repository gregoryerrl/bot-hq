// src/lib/plugins/mcp-manager.ts

import { spawn, ChildProcess } from "child_process";
import { join } from "path";
import { LoadedPlugin } from "./types";
import { getPluginRegistry } from "./registry";

interface McpServer {
  plugin: LoadedPlugin;
  process: ChildProcess | null;
  status: "stopped" | "starting" | "running" | "error";
  pendingCalls: Map<string, {
    resolve: (value: unknown) => void;
    reject: (error: Error) => void;
  }>;
  callId: number;
}

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

    const server: McpServer = {
      plugin,
      process: null,
      status: "starting",
      pendingCalls: new Map(),
      callId: 0,
    };
    this.servers.set(plugin.name, server);

    const entryPath = join(plugin.path, plugin.manifest.mcp.entry);

    // Get credentials from registry to pass as env
    const registry = getPluginRegistry();
    const credentials = await registry.getCredentials(plugin.name);

    const env = {
      ...process.env,
      ...credentials,
    };

    // Spawn the MCP server process
    const child = spawn("npx", ["tsx", entryPath], {
      cwd: plugin.path,
      env,
      stdio: ["pipe", "pipe", "pipe"],
    });

    server.process = child;

    let buffer = "";

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
      console.error(`[${plugin.name}] MCP stderr:`, data.toString());
    });

    child.on("exit", (code) => {
      console.log(`[${plugin.name}] MCP server exited with code ${code}`);
      server.status = "stopped";
      server.process = null;

      // Reject any pending calls
      for (const [, { reject }] of server.pendingCalls) {
        reject(new Error(`MCP server exited with code ${code}`));
      }
      server.pendingCalls.clear();
    });

    child.on("error", (error) => {
      console.error(`[${plugin.name}] MCP server error:`, error);
      server.status = "error";
    });

    // Wait for server to be ready (simplified - real impl would wait for initialize response)
    await new Promise(resolve => setTimeout(resolve, 1000));
    server.status = "running";

    console.log(`[${plugin.name}] MCP server started`);
  }

  async stopServer(pluginName: string): Promise<void> {
    const server = this.servers.get(pluginName);
    if (!server?.process) return;

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
        throw new Error(`Plugin ${pluginName} not found`);
      }
      await this.startServer(plugin);
      return this.callTool(pluginName, tool, params);
    }

    if (server.status !== "running") {
      throw new Error(`MCP server for ${pluginName} is not running`);
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
      server.pendingCalls.set(id, { resolve, reject });
      server.process?.stdin?.write(JSON.stringify(request) + "\n");

      // Timeout after 30 seconds
      setTimeout(() => {
        if (server.pendingCalls.has(id)) {
          server.pendingCalls.delete(id);
          reject(new Error(`MCP call to ${tool} timed out`));
        }
      }, 30000);
    });
  }

  private handleMessage(pluginName: string, message: { id?: string; result?: unknown; error?: { message: string } }): void {
    const server = this.servers.get(pluginName);
    if (!server) return;

    if (message.id) {
      const pending = server.pendingCalls.get(message.id);
      if (pending) {
        server.pendingCalls.delete(message.id);
        if (message.error) {
          pending.reject(new Error(message.error.message));
        } else {
          pending.resolve(message.result);
        }
      }
    }
  }

  getServerStatus(pluginName: string): string {
    return this.servers.get(pluginName)?.status || "not_loaded";
  }

  async stopAll(): Promise<void> {
    for (const [name] of this.servers) {
      await this.stopServer(name);
    }
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
