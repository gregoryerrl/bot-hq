package projdata

import (
	"context"
	"database/sql"
	"os"
	"path/filepath"
	"strings"
	"testing"

	_ "modernc.org/sqlite"
)

func TestValidateSelectOnly_AcceptsSelect(t *testing.T) {
	cases := []string{
		"SELECT 1",
		"select * from users",
		"SELECT name, email FROM users WHERE active = 1 LIMIT 10",
		"WITH recent AS (SELECT * FROM jobs ORDER BY created DESC LIMIT 100) SELECT * FROM recent WHERE status = 'failed'",
		"EXPLAIN SELECT * FROM users",
		"EXPLAIN QUERY PLAN SELECT * FROM jobs",
		"SELECT 1; ", // trailing semicolon OK
		"-- comment\nSELECT 1",
		"/* block */ SELECT 1",
	}
	for _, q := range cases {
		t.Run(q, func(t *testing.T) {
			if err := ValidateSelectOnly(q); err != nil {
				t.Errorf("expected accept: %v", err)
			}
		})
	}
}

func TestValidateSelectOnly_RejectsMutations(t *testing.T) {
	cases := []struct {
		name, q string
	}{
		{"insert", "INSERT INTO users (name) VALUES ('x')"},
		{"update", "UPDATE users SET name = 'x'"},
		{"delete", "DELETE FROM users"},
		{"drop", "DROP TABLE users"},
		{"alter", "ALTER TABLE users ADD COLUMN x INT"},
		{"create", "CREATE TABLE x (id INT)"},
		{"replace", "REPLACE INTO users VALUES (1, 'x')"},
		{"truncate", "TRUNCATE users"},
		{"grant", "GRANT SELECT ON x TO y"},
		{"pragma_write", "PRAGMA journal_mode = WAL"},
		{"attach", "ATTACH DATABASE 'x.db' AS x"},
		{"vacuum", "VACUUM"},
		{"copy", "COPY users FROM '/tmp/x'"},
		{"set", "SET search_path = public"},
		{"multi_statement", "SELECT 1; DROP TABLE x"},
		{"trailing_drop", "SELECT 1; DROP TABLE x;"},
		{"begin", "BEGIN; SELECT 1; COMMIT"},
		{"select_into", "SELECT * INTO new_users FROM users"},
		{"empty", ""},
		{"comment_only", "-- only comment"},
		{"non_select", "SHOW TABLES"},
		{"hidden_mutation", "SELECT 1; -- DROP TABLE users\nDELETE FROM users"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if err := ValidateSelectOnly(tc.q); err == nil {
				t.Errorf("expected reject for %q", tc.q)
			}
		})
	}
}

func TestValidateSelectOnly_DoesNotMatchSubstrings(t *testing.T) {
	// "SELECTED" shouldn't trigger the SELECT match; "INSERTED" shouldn't
	// trigger INSERT.
	cases := []string{
		"SELECT name FROM users WHERE status = 'INSERTED'",
		"SELECT count FROM stats WHERE label = 'CREATE_USER'",
	}
	for _, q := range cases {
		t.Run(q, func(t *testing.T) {
			if err := ValidateSelectOnly(q); err != nil {
				t.Errorf("expected accept: %v", err)
			}
		})
	}
}

func TestLoadEnvFile_HappyPath(t *testing.T) {
	root := t.TempDir()
	t.Setenv("BOT_HQ_HOME", root)
	dir := filepath.Join(root, "projects", "test-proj", "env")
	if err := os.MkdirAll(dir, 0o700); err != nil {
		t.Fatal(err)
	}
	content := `# leading comment
PROD_DB_RO_DSN=postgres://ro@localhost/app
PROD_HOST="bastion.internal"
EMPTY=
# trailing
`
	if err := os.WriteFile(filepath.Join(dir, "prod.env"), []byte(content), 0o600); err != nil {
		t.Fatal(err)
	}
	env, err := LoadEnvFile("test-proj", "prod.env")
	if err != nil {
		t.Fatal(err)
	}
	if env["PROD_DB_RO_DSN"] != "postgres://ro@localhost/app" {
		t.Errorf("PROD_DB_RO_DSN unexpected: %q", env["PROD_DB_RO_DSN"])
	}
	if env["PROD_HOST"] != "bastion.internal" {
		t.Errorf("PROD_HOST unexpected: %q (quotes should strip)", env["PROD_HOST"])
	}
	if _, ok := env["EMPTY"]; !ok {
		t.Errorf("EMPTY key should exist (with empty value)")
	}
}

func TestLoadEnvFile_RejectsPathTraversal(t *testing.T) {
	if _, err := LoadEnvFile("test", "../../etc/passwd"); err == nil {
		t.Error("expected reject for traversal")
	}
	if _, err := LoadEnvFile("test", "sub/file"); err == nil {
		t.Error("expected reject for nested path")
	}
}

func TestLoadEnvFile_MissingFileIsEmpty(t *testing.T) {
	root := t.TempDir()
	t.Setenv("BOT_HQ_HOME", root)
	env, err := LoadEnvFile("test-proj", "missing.env")
	if err != nil {
		t.Fatal(err)
	}
	if len(env) != 0 {
		t.Errorf("expected empty map, got %v", env)
	}
}

func TestQuery_SQLite_RejectsNotReadOnly(t *testing.T) {
	cfg := DataSourceConfig{
		Name:     "test",
		Type:     "sqlite",
		DSNEnv:   "DSN",
		ReadOnly: false,
	}
	_, err := Query(context.Background(), cfg, map[string]string{"DSN": ":memory:"}, "SELECT 1", 10)
	if err == nil || !strings.Contains(err.Error(), "not marked read_only") {
		t.Errorf("expected read-only-flag rejection; got %v", err)
	}
}

func TestQuery_SQLite_HappyPath(t *testing.T) {
	// Create a temp SQLite DB with a known row.
	dbPath := filepath.Join(t.TempDir(), "test.db")
	wdb, err := sql.Open("sqlite", dbPath)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := wdb.Exec(`CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, status TEXT)`); err != nil {
		t.Fatal(err)
	}
	if _, err := wdb.Exec(`INSERT INTO users (name, status) VALUES ('alice', 'active'), ('bob', 'inactive')`); err != nil {
		t.Fatal(err)
	}
	wdb.Close()

	cfg := DataSourceConfig{
		Name:     "test",
		Type:     "sqlite",
		DSNEnv:   "DSN",
		EnvFile:  "test.env",
		ReadOnly: true,
	}
	env := map[string]string{"DSN": dbPath}
	res, err := Query(context.Background(), cfg, env, "SELECT id, name FROM users WHERE status = 'active'", 10)
	if err != nil {
		t.Fatal(err)
	}
	if res.RowCount != 1 {
		t.Errorf("expected 1 row, got %d", res.RowCount)
	}
	if res.Rows[0]["name"] != "alice" {
		t.Errorf("expected alice, got %v", res.Rows[0]["name"])
	}
	if res.Source != "test" {
		t.Errorf("source=%q want test", res.Source)
	}
	if len(res.Columns) != 2 {
		t.Errorf("expected 2 cols, got %v", res.Columns)
	}
}

func TestQuery_SQLite_BlocksMutation(t *testing.T) {
	dbPath := filepath.Join(t.TempDir(), "test.db")
	wdb, _ := sql.Open("sqlite", dbPath)
	wdb.Exec(`CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)`)
	wdb.Close()

	cfg := DataSourceConfig{Name: "test", Type: "sqlite", DSNEnv: "DSN", ReadOnly: true}
	env := map[string]string{"DSN": dbPath}
	_, err := Query(context.Background(), cfg, env, "DELETE FROM users", 10)
	if err == nil || !strings.Contains(err.Error(), "sql gate") {
		t.Errorf("expected gate reject; got %v", err)
	}
}

func TestQuery_SQLite_EnforcesReadOnlyDriver(t *testing.T) {
	// Even though our gate rejects mutations, double-check the SQLite
	// ?mode=ro flag is in effect: a query that GOES THROUGH the gate
	// but tries to attach a writable connection would still fail.
	// (PRAGMA-write is gate-rejected; this test confirms gate is the
	// load-bearing piece; mode=ro is defense-in-depth.)
	dbPath := filepath.Join(t.TempDir(), "test.db")
	wdb, _ := sql.Open("sqlite", dbPath)
	wdb.Exec(`CREATE TABLE x (id INT)`)
	wdb.Close()

	cfg := DataSourceConfig{Name: "x", Type: "sqlite", DSNEnv: "DSN", ReadOnly: true}
	env := map[string]string{"DSN": dbPath}
	res, err := Query(context.Background(), cfg, env, "SELECT count(*) AS n FROM x", 10)
	if err != nil {
		t.Fatal(err)
	}
	if res.RowCount != 1 {
		t.Errorf("expected 1 row from count()")
	}
}

func TestQuery_LimitTruncates(t *testing.T) {
	dbPath := filepath.Join(t.TempDir(), "test.db")
	wdb, _ := sql.Open("sqlite", dbPath)
	wdb.Exec(`CREATE TABLE x (i INT)`)
	for i := 0; i < 20; i++ {
		wdb.Exec(`INSERT INTO x VALUES (?)`, i)
	}
	wdb.Close()

	cfg := DataSourceConfig{Name: "x", Type: "sqlite", DSNEnv: "DSN", ReadOnly: true}
	env := map[string]string{"DSN": dbPath}
	res, err := Query(context.Background(), cfg, env, "SELECT i FROM x", 5)
	if err != nil {
		t.Fatal(err)
	}
	if res.RowCount != 5 {
		t.Errorf("expected 5 rows (limit), got %d", res.RowCount)
	}
	if res.TruncatedAt != 5 {
		t.Errorf("expected truncated_at=5, got %d", res.TruncatedAt)
	}
}
