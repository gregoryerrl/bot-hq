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

	_ "github.com/lib/pq"
	_ "modernc.org/sqlite"
)

// normalizeDriverType maps Laravel-style DB_CONNECTION values + common
// aliases to bot-hq's canonical driver names.
func normalizeDriverType(s string) string {
	switch strings.ToLower(strings.TrimSpace(s)) {
	case "pgsql", "postgres", "postgresql":
		return "postgres"
	case "mysql", "mariadb":
		return "mysql"
	case "sqlite", "sqlite3":
		return "sqlite"
	default:
		return strings.ToLower(strings.TrimSpace(s))
	}
}

// buildPostgresDSN assembles a postgres URL from components. Password is
// URL-encoded so special chars (e.g., @) don't break parsing.
func buildPostgresDSN(host, port, db, user, pass, sslmode string) string {
	if sslmode == "" {
		sslmode = "prefer"
	}
	hostport := host
	if port != "" {
		hostport = host + ":" + port
	}
	userinfo := url.UserPassword(user, pass).String()
	return "postgres://" + userinfo + "@" + hostport + "/" + db + "?sslmode=" + sslmode
}

// buildMySQLDSN assembles a mysql DSN per go-sql-driver/mysql format:
// user:pass@tcp(host:port)/db?...
func buildMySQLDSN(host, port, db, user, pass string) string {
	if port == "" {
		port = "3306"
	}
	return user + ":" + pass + "@tcp(" + host + ":" + port + ")/" + db + "?parseTime=true"
}

// DataSourceConfig is one entry in a project yaml's `data_sources.databases`
// list. Loaded by Load() + ResolveSource().
//
// Two credential modes (yaml chooses one):
//   - dsn_env: single env-var name holding a complete DSN string (e.g.,
//     PROD_DB_RO_DSN=postgres://user:pass@host:5432/db?sslmode=require).
//     Used when the operator already has a DSN-shaped secret.
//   - Component env-vars: host_env / port_env / database_env / username_env
//     / password_env each name an env-var holding one piece. Useful for
//     Laravel-style .env files (DB_HOST / DB_PORT / DB_DATABASE / etc.)
//     where bot-hq can reuse the same env file the app already uses.
type DataSourceConfig struct {
	Name                string `yaml:"name"`
	Type                string `yaml:"type"`     // sqlite | postgres | mysql; auto-derived from DB_CONNECTION if empty + ConnectionEnv set
	DSNEnv              string `yaml:"dsn_env"`  // env-var name resolved from EnvFile
	EnvFile             string `yaml:"env_file"` // file name under projects/<project>/env/
	ReadOnly            bool   `yaml:"read_only"`
	QueryTimeoutSeconds int    `yaml:"query_timeout_seconds"`

	// Component env-vars (alternative to dsn_env; used when env file is
	// Laravel-style with discrete host/port/user/pass/db pieces).
	ConnectionEnv string `yaml:"connection_env"` // env-var name (e.g., DB_CONNECTION → "pgsql"/"mysql"/"sqlite")
	HostEnv       string `yaml:"host_env"`
	PortEnv       string `yaml:"port_env"`
	DatabaseEnv   string `yaml:"database_env"`
	UsernameEnv   string `yaml:"username_env"`
	PasswordEnv   string `yaml:"password_env"`
	SSLMode       string `yaml:"sslmode"` // postgres-specific; defaults to "prefer"
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

	// Resolve driver type — yaml `type` wins; else look at ConnectionEnv.
	dbType := normalizeDriverType(cfg.Type)
	if dbType == "" && cfg.ConnectionEnv != "" {
		dbType = normalizeDriverType(env[cfg.ConnectionEnv])
	}
	if dbType == "" {
		return nil, fmt.Errorf("data source %q has no driver type (yaml `type:` or env %s required)", cfg.Name, cfg.ConnectionEnv)
	}

	// Resolve DSN — single-DSN mode wins; else build from components.
	var dsn string
	if cfg.DSNEnv != "" {
		v, ok := env[cfg.DSNEnv]
		if !ok || v == "" {
			return nil, fmt.Errorf("env var %s not set in env file %s", cfg.DSNEnv, cfg.EnvFile)
		}
		dsn = v
	} else if cfg.HostEnv != "" {
		host := env[cfg.HostEnv]
		port := env[cfg.PortEnv]
		database := env[cfg.DatabaseEnv]
		user := env[cfg.UsernameEnv]
		pass := env[cfg.PasswordEnv]
		if host == "" || database == "" || user == "" {
			return nil, fmt.Errorf("component env vars incomplete (need at minimum host/database/username; got host=%q database=%q username=%q)", redact(host), redact(database), redact(user))
		}
		switch dbType {
		case "postgres":
			dsn = buildPostgresDSN(host, port, database, user, pass, cfg.SSLMode)
		case "mysql":
			dsn = buildMySQLDSN(host, port, database, user, pass)
		case "sqlite":
			return nil, fmt.Errorf("sqlite does not support component env vars; use dsn_env with a file path")
		default:
			return nil, fmt.Errorf("unsupported driver %q for component-env DSN build", dbType)
		}
	} else {
		return nil, fmt.Errorf("data source %q has neither dsn_env nor component env vars (host_env/port_env/database_env/username_env/password_env)", cfg.Name)
	}

	driverName, dsnReady, err := prepareDSN(dbType, dsn)
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
	switch dbType {
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

// redact replaces all characters of a string with X for safe display
// in errors when the value might be PII or a credential.
func redact(s string) string {
	if s == "" {
		return ""
	}
	return strings.Repeat("X", len(s))
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
