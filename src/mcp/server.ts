import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { registerAgentTools } from "./tools/agents.js";
import { registerTaskTools } from "./tools/tasks.js";
import { registerMonitoringTools } from "./tools/monitoring.js";

const server = new McpServer({
  name: "bot-hq",
  version: "1.0.0",
});

// Register all tools
registerAgentTools(server);
registerTaskTools(server);
registerMonitoringTools(server);

// Start with stdio transport
async function main() {
  const transport = new StdioServerTransport();
  await server.connect(transport);
  console.error("[bot-hq MCP] Server started");
}

main().catch((error) => {
  console.error("[bot-hq MCP] Fatal error:", error);
  process.exit(1);
});
