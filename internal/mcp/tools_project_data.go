package mcp

import (
	"context"
	"fmt"
	"strings"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/projdata"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
	"github.com/mark3labs/mcp-go/mcp"
)

// hubProjectQuery is the Z-4-a MCP tool that lets agents query a
// project's allowlisted data source(s) read-only during Investigate
// phase. See internal/projdata for the load + gate + execute pipeline.
//
// Audit trail: every fire emits a [DATA-QUERY] hub message with
// caller / project / source / row-count / elapsed for forensics. Query
// content itself is NOT emitted to hub.db (PII / secret leakage risk).
func hubProjectQuery(db *hub.DB) ToolDef {
	tool := mcp.NewTool("bot_hq_project_query",
		mcp.WithDescription("Z-4-a: query a project's allowlisted data source read-only. Source name + project must match projects/<project>.yaml `data_sources.databases` block. Credentials resolve from ~/.bot-hq/projects/<project>/env/<env_file>. SELECT-only enforced via SQL gate (rejects INSERT/UPDATE/DELETE/DROP/etc.). Use during Investigate phase to inspect prod state, logs-as-DB, or audit trails."),
		mcp.WithString("from", mcp.Required(), mcp.Description("Caller agent ID (for audit emit)")),
		mcp.WithString("project", mcp.Required(), mcp.Description("Project key (matches projects/<key>.yaml)")),
		mcp.WithString("source", mcp.Required(), mcp.Description("Data source name from yaml `data_sources.databases.name`")),
		mcp.WithString("query", mcp.Required(), mcp.Description("SELECT-only SQL (rejected at SQL gate: INSERT/UPDATE/DELETE/DROP/ALTER/CREATE/REPLACE/TRUNCATE/GRANT/PRAGMA-write/ATTACH/VACUUM/COPY/SET/BEGIN/COMMIT/multi-statement). WITH-CTE + EXPLAIN allowed.")),
		mcp.WithNumber("limit", mcp.Description("Max rows to return (default 100, max 5000). Result truncates with truncated_at marker if hit.")),
	)

	handler := func(ctx context.Context, req mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		from, err := req.RequireString("from")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		project, err := req.RequireString("project")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		source, err := req.RequireString("source")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		query, err := req.RequireString("query")
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}
		limit := req.GetInt("limit", 100)

		// Load project config + resolve named source via allowlist.
		cfg, err := projdata.LoadConfig(project)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("project config: %v", err)), nil
		}
		src, err := projdata.ResolveSource(cfg, source)
		if err != nil {
			return mcp.NewToolResultError(err.Error()), nil
		}

		// Load env-file for credentials.
		env, err := projdata.LoadEnvFile(project, src.EnvFile)
		if err != nil {
			return mcp.NewToolResultError(fmt.Sprintf("env load: %v", err)), nil
		}

		// Execute (SQL gate + driver-level RO inside Query).
		t0 := time.Now()
		result, err := projdata.Query(ctx, src, env, query, limit)
		elapsed := time.Since(t0).Milliseconds()
		if err != nil {
			// Audit the failure too — keep query text out, log error class only.
			_ = emitDataQueryAudit(db, from, project, source, 0, elapsed, "fail:"+strings.SplitN(err.Error(), ":", 2)[0])
			return mcp.NewToolResultError(fmt.Sprintf("query: %v", err)), nil
		}

		_ = emitDataQueryAudit(db, from, project, source, result.RowCount, result.ElapsedMs, "ok")
		return mcp.NewToolResultText(toJSON(result)), nil
	}

	return ToolDef{Tool: tool, Handler: handler}
}

// emitDataQueryAudit writes a [DATA-QUERY] hub message tracking who
// queried what (no query text + no result rows to keep secrets out of
// hub.db). Forensics-only.
func emitDataQueryAudit(db *hub.DB, from, project, source string, rowCount int, elapsedMs int64, status string) error {
	content := fmt.Sprintf("[DATA-QUERY] from=%s project=%s source=%s status=%s rows=%d elapsed_ms=%d",
		from, project, source, status, rowCount, elapsedMs)
	_, err := db.InsertMessage(protocol.Message{
		FromAgent: from,
		Type:      protocol.MsgUpdate,
		Content:   content,
	})
	return err
}
