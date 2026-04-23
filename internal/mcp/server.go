package mcp

import (
	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
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
