// Package projdata — Z-4-a per-project data-source access.
//
// Lets agents query a project's prod-class data sources (databases,
// logs) read-only during the Investigate phase of an IPAV cycle.
// Credentials live in per-project CL env files
// (~/.bot-hq/projects/<project>/env/<file>.env) referenced by the
// project's yaml `data_sources` block — yaml is committed; env files
// are gitignored secrets-class.
//
// Security:
//   - Allowlist-only: source name must match yaml; agents can't supply
//     arbitrary paths or DSNs.
//   - SELECT-only SQL gate (sqlgate.go) rejects any mutation keyword.
//   - Connections open in read-only mode where the driver supports it
//     (SQLite: ?mode=ro; Postgres: SET default_transaction_read_only).
//   - Query timeout enforced; row count capped.
//   - Every fire emits an audit message to hub for forensics.
package projdata

import (
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"time"

	_ "modernc.org/sqlite"
)

// DataSourceConfig is one entry in a project yaml's `data_sources.databases`
// list. Loaded by Load() + ResolveSource().
type DataSourceConfig struct {
	Name                string `yaml:"name"`
	Type                string `yaml:"type"`     // sqlite | postgres | mysql
	DSNEnv              string `yaml:"dsn_env"`  // env-var name resolved from EnvFile
	EnvFile             string `yaml:"env_file"` // file name under projects/<project>/env/
	ReadOnly            bool   `yaml:"read_only"`
	QueryTimeoutSeconds int    `yaml:"query_timeout_seconds"`
}

// ProjectDataSources groups all data-source entries for one project.
type ProjectDataSources struct {
	Databases []DataSourceConfig `yaml:"databases"`
}

// envDir returns the path under which a project's env files live.
// Honors BOT_HQ_HOME for test isolation.
func envDir(project string) string {
	root := os.Getenv("BOT_HQ_HOME")
	if root == "" {
		home, _ := os.UserHomeDir()
		root = filepath.Join(home, ".bot-hq")
	}
	return filepath.Join(root, "projects", project, "env")
}

// LoadEnvFile reads a project's env-file as KEY=VALUE pairs. Comments
// (#-leading) and blank lines are ignored. Returns map keyed by var name.
//
// Missing file is not an error — returns an empty map. Caller decides
// whether the absence is a problem (e.g., a required dsn_env not present
// is the actual issue).
func LoadEnvFile(project, filename string) (map[string]string, error) {
	out := map[string]string{}
	if project == "" || filename == "" {
		return out, nil
	}
	// Allow only simple file names — reject path traversal.
	if strings.ContainsAny(filename, "/\\") || strings.Contains(filename, "..") {
		return out, fmt.Errorf("env file name %q invalid (no slashes or ..)", filename)
	}
	path := filepath.Join(envDir(project), filename)
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return out, nil
		}
		return out, fmt.Errorf("read env file %s: %w", path, err)
	}
	for _, raw := range strings.Split(string(data), "\n") {
		line := strings.TrimSpace(raw)
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		eq := strings.IndexByte(line, '=')
		if eq < 0 {
			continue
		}
		key := strings.TrimSpace(line[:eq])
		val := strings.TrimSpace(line[eq+1:])
		// Strip surrounding double-quotes if present.
		if len(val) >= 2 && val[0] == '"' && val[len(val)-1] == '"' {
			val = val[1 : len(val)-1]
		}
		out[key] = val
	}
	return out, nil
}

// QueryResult is the JSON-friendly result of a SELECT.
type QueryResult struct {
	Source       string           `json:"source"`
	Columns      []string         `json:"columns"`
	Rows         []map[string]any `json:"rows"`
	RowCount     int              `json:"row_count"`
	TruncatedAt  int              `json:"truncated_at,omitempty"`
	ElapsedMs    int64            `json:"elapsed_ms"`
}

// Query opens a DB connection for the given DataSourceConfig + env-vars
// map, validates the query via SQL gate, executes with timeout, and
// returns results capped at limit rows.
//
// Caller is responsible for resolving DataSourceConfig + env map (typical
// flow: yaml load → LoadEnvFile → Query).
func Query(ctx context.Context, cfg DataSourceConfig, env map[string]string, query string, limit int) (*QueryResult, error) {
	if !cfg.ReadOnly {
		return nil, fmt.Errorf("data source %q is not marked read_only: true in yaml — refuse to open", cfg.Name)
	}
	if err := ValidateSelectOnly(query); err != nil {
		return nil, fmt.Errorf("sql gate reject: %w", err)
	}
	dsn, ok := env[cfg.DSNEnv]
	if !ok || dsn == "" {
		return nil, fmt.Errorf("env var %s not set in env file %s", cfg.DSNEnv, cfg.EnvFile)
	}

	driverName, dsnReady, err := prepareDSN(cfg.Type, dsn)
	if err != nil {
		return nil, err
	}
	db, err := sql.Open(driverName, dsnReady)
	if err != nil {
		return nil, fmt.Errorf("open %s: %w", cfg.Type, err)
	}
	defer db.Close()

	timeout := cfg.QueryTimeoutSeconds
	if timeout <= 0 {
		timeout = 30
	}
	qctx, cancel := context.WithTimeout(ctx, time.Duration(timeout)*time.Second)
	defer cancel()

	// Postgres / MySQL: set session-level RO at connection open. SQLite
	// honors via ?mode=ro in prepareDSN.
	switch cfg.Type {
	case "postgres":
		if _, err := db.ExecContext(qctx, "SET default_transaction_read_only = on"); err != nil {
			return nil, fmt.Errorf("set ro: %w", err)
		}
	case "mysql":
		if _, err := db.ExecContext(qctx, "SET SESSION TRANSACTION READ ONLY"); err != nil {
			return nil, fmt.Errorf("set ro: %w", err)
		}
	}

	if limit <= 0 {
		limit = 100
	}
	if limit > 5000 {
		limit = 5000
	}

	t0 := time.Now()
	rows, err := db.QueryContext(qctx, query)
	if err != nil {
		return nil, fmt.Errorf("query: %w", err)
	}
	defer rows.Close()

	cols, err := rows.Columns()
	if err != nil {
		return nil, fmt.Errorf("columns: %w", err)
	}

	result := &QueryResult{
		Source:  cfg.Name,
		Columns: cols,
	}
	for rows.Next() {
		if len(result.Rows) >= limit {
			result.TruncatedAt = limit
			break
		}
		vals := make([]any, len(cols))
		ptrs := make([]any, len(cols))
		for i := range vals {
			ptrs[i] = &vals[i]
		}
		if err := rows.Scan(ptrs...); err != nil {
			return nil, fmt.Errorf("scan: %w", err)
		}
		row := make(map[string]any, len(cols))
		for i, c := range cols {
			row[c] = normalizeValue(vals[i])
		}
		result.Rows = append(result.Rows, row)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("rows: %w", err)
	}
	result.RowCount = len(result.Rows)
	result.ElapsedMs = time.Since(t0).Milliseconds()
	return result, nil
}

// prepareDSN normalizes a DSN for the chosen driver + enforces read-only
// where the driver supports a URL-level flag (SQLite ?mode=ro).
func prepareDSN(dbType, dsn string) (driverName, dsnReady string, err error) {
	switch dbType {
	case "sqlite", "sqlite3":
		// SQLite: ?mode=ro forces read-only at open-time. Also disable
		// WAL writes etc. Path-based DSN: prefix file: + query string.
		path := dsn
		path = strings.TrimPrefix(path, "sqlite://")
		path = strings.TrimPrefix(path, "file:")
		// Expand ~ at start.
		if strings.HasPrefix(path, "~/") {
			home, _ := os.UserHomeDir()
			path = filepath.Join(home, path[2:])
		}
		// Allow ?param= suffix on the path.
		extraQuery := ""
		if i := strings.IndexByte(path, '?'); i >= 0 {
			extraQuery = path[i+1:]
			path = path[:i]
		}
		q := url.Values{}
		if extraQuery != "" {
			parsed, perr := url.ParseQuery(extraQuery)
			if perr == nil {
				q = parsed
			}
		}
		q.Set("mode", "ro")
		q.Set("_busy_timeout", "5000")
		return "sqlite", "file:" + path + "?" + q.Encode(), nil
	case "postgres", "postgresql":
		// Caller must supply readonly user creds in the DSN. We also
		// SET default_transaction_read_only after open.
		return "postgres", dsn, nil
	case "mysql":
		// Same — readonly creds expected. SET SESSION TRANSACTION
		// READ ONLY after open.
		return "mysql", dsn, nil
	default:
		return "", "", fmt.Errorf("unsupported data source type %q (supported: sqlite, postgres, mysql)", dbType)
	}
}

// normalizeValue converts driver-returned []byte to string for JSON
// friendliness. Other types pass through unchanged.
func normalizeValue(v any) any {
	if b, ok := v.([]byte); ok {
		return string(b)
	}
	return v
}

// MarshalJSON wraps QueryResult JSON output for consistent shape.
func (r *QueryResult) MarshalJSON() ([]byte, error) {
	type Alias QueryResult
	return json.Marshal((*Alias)(r))
}
