"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
const mcp_js_1 = require("@modelcontextprotocol/sdk/server/mcp.js");
const stdio_js_1 = require("@modelcontextprotocol/sdk/server/stdio.js");
const agents_js_1 = require("./tools/agents.js");
const tasks_js_1 = require("./tools/tasks.js");
const approvals_js_1 = require("./tools/approvals.js");
const monitoring_js_1 = require("./tools/monitoring.js");
const server = new mcp_js_1.McpServer({
    name: "bot-hq",
    version: "1.0.0",
});
// Register all tools
(0, agents_js_1.registerAgentTools)(server);
(0, tasks_js_1.registerTaskTools)(server);
(0, approvals_js_1.registerApprovalTools)(server);
(0, monitoring_js_1.registerMonitoringTools)(server);
// Start with stdio transport
async function main() {
    const transport = new stdio_js_1.StdioServerTransport();
    await server.connect(transport);
    console.error("[bot-hq MCP] Server started");
}
main().catch((error) => {
    console.error("[bot-hq MCP] Fatal error:", error);
    process.exit(1);
});
