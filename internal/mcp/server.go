package mcp

import (
	"context"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
	"github.com/mark3labs/mcp-go/mcp"
	"github.com/mark3labs/mcp-go/server"
)

// RunStdioServer creates an MCP server with all hub tools and serves on stdio.
func RunStdioServer(db *hub.DB) error {
	s := server.NewMCPServer("bot-hq", protocol.Version)

	for _, td := range BuildTools(db) {
		s.AddTool(td.Tool, td.Handler)
	}

	return server.ServeStdio(s)
}

// RunStdioServerWithError starts a minimal MCP server that responds to any
// tool call with the given error message. This allows MCP clients to receive
// a proper JSON-RPC error instead of an unexpected EOF when the server
// cannot fully initialize (e.g. database unavailable).
func RunStdioServerWithError(errMsg string) {
	s := server.NewMCPServer("bot-hq", protocol.Version)

	errorTool := mcp.NewTool("check_status",
		mcp.WithDescription("Check bot-hq server status"),
	)
	s.AddTool(errorTool, func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		return mcp.NewToolResultError(errMsg), nil
	})

	server.ServeStdio(s)
}
