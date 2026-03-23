import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { registerProjectTools } from "./tools/projects.js";
import { registerTaskTools } from "./tools/tasks.js";
import { registerDiagramTools } from "./tools/diagrams.js";
import { registerSummaryTools } from "./tools/summary.js";

const server = new McpServer({
  name: "bot-hq",
  version: "2.0.0",
});

// Register all tools
registerProjectTools(server);
registerTaskTools(server);
registerDiagramTools(server);
registerSummaryTools(server);

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
