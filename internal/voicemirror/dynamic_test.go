package voicemirror

import (
	"database/sql"
	"os"
	"path/filepath"
	"testing"

	_ "modernc.org/sqlite"
)

func TestExtractPathsFromContent_BasicAbsolute(t *testing.T) {
	got := extractPathsFromContent("please edit /tmp/scratch.md and run tests")
	if len(got) != 1 || got[0] != "/tmp/scratch.md" {
		t.Errorf("expected [/tmp/scratch.md], got %v", got)
	}
}

func TestExtractPathsFromContent_TildeExpansion(t *testing.T) {
	home, _ := os.UserHomeDir()
	got := extractPathsFromContent("write to ~/Documents/notes.md please")
	want := filepath.Join(home, "Documents/notes.md")
	if len(got) != 1 || got[0] != want {
		t.Errorf("expected [%s], got %v", want, got)
	}
}

func TestExtractPathsFromContent_HOMEExpansion(t *testing.T) {
	home, _ := os.UserHomeDir()
	got := extractPathsFromContent("touch $HOME/foo/bar.txt")
	want := filepath.Join(home, "foo/bar.txt")
	if len(got) != 1 || got[0] != want {
		t.Errorf("expected [%s], got %v", want, got)
	}
}

func TestExtractPathsFromContent_NoExtensionSkipped(t *testing.T) {
	got := extractPathsFromContent("the API at http://localhost:3000 works")
	if len(got) != 0 {
		t.Errorf("expected no matches (no path extension), got %v", got)
	}
}

func TestExtractPathsFromContent_SpaceInPathMisses(t *testing.T) {
	home, _ := os.UserHomeDir()
	got := extractPathsFromContent("edit ~/My Folder/x.md please")
	wantFull := filepath.Join(home, "My Folder/x.md")
	for _, p := range got {
		if p == wantFull {
			t.Errorf("space-in-path should not match full path (documented limitation): got %v", got)
		}
	}
	// Documented behavior: regex may extract partial sub-path
	// after the space (e.g., "/x.md"); we accept this and log
	// as known limitation rather than over-engineering the regex.
}

func TestExtractPathsFromContent_Dedupes(t *testing.T) {
	got := extractPathsFromContent("edit /tmp/a.md and also /tmp/a.md again")
	if len(got) != 1 {
		t.Errorf("expected dedupe, got %v", got)
	}
}

func TestExtractPathsFromContent_MultiplePaths(t *testing.T) {
	got := extractPathsFromContent("touch /a/x.go and ~/b/y.md")
	if len(got) != 2 {
		t.Errorf("expected 2 paths, got %v", got)
	}
}

func TestMatchesDynamicInclude_ExactMatch(t *testing.T) {
	if !MatchesDynamicInclude("/tmp/x.md", []string{"/tmp/x.md"}) {
		t.Error("expected exact match")
	}
}

func TestMatchesDynamicInclude_SubPathPrefix(t *testing.T) {
	if !MatchesDynamicInclude("/tmp/dir/inner.md", []string{"/tmp/dir"}) {
		t.Error("expected sub-path match for /tmp/dir prefix")
	}
}

func TestMatchesDynamicInclude_PartialNameOverlapNoMatch(t *testing.T) {
	if MatchesDynamicInclude("/tmp/dir-other.md", []string{"/tmp/dir"}) {
		t.Error("partial-name overlap should not match (separator-anchored prefix)")
	}
}

func TestMatchesDynamicInclude_EmptyDynamicPaths(t *testing.T) {
	if MatchesDynamicInclude("/tmp/x.md", nil) {
		t.Error("empty dynamic-paths should never match")
	}
}

// setupTestHubDB creates an isolated test sqlite3 hub.db with messages
// schema matching internal/hub/db.go expectations. Returns path.
func setupTestHubDB(t *testing.T) string {
	t.Helper()
	dir := t.TempDir()
	path := filepath.Join(dir, "hub.db")
	db, err := sql.Open("sqlite", path)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	_, err = db.Exec(`
		CREATE TABLE messages (
			id INTEGER PRIMARY KEY AUTOINCREMENT,
			from_agent TEXT,
			to_agent TEXT,
			content TEXT,
			type TEXT,
			created TEXT
		)
	`)
	if err != nil {
		t.Fatal(err)
	}
	return path
}

func insertUserMsg(t *testing.T, dbPath, content string) {
	t.Helper()
	db, err := sql.Open("sqlite", dbPath)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	if _, err := db.Exec(
		`INSERT INTO messages(from_agent, content) VALUES('user', ?)`,
		content,
	); err != nil {
		t.Fatal(err)
	}
}

func TestCollectDynamicPaths_DBUnavailable(t *testing.T) {
	t.Setenv(hubDBPathEnvVar, "/nonexistent/path/hub.db")
	got := collectDynamicPaths()
	if got != nil {
		t.Errorf("DB-unavailable should fail-open with nil, got %v", got)
	}
}

func TestCollectDynamicPaths_EmptyDB(t *testing.T) {
	path := setupTestHubDB(t)
	t.Setenv(hubDBPathEnvVar, path)
	got := collectDynamicPaths()
	if len(got) != 0 {
		t.Errorf("empty DB should produce zero paths, got %v", got)
	}
}

func TestCollectDynamicPaths_SingleUserMsg(t *testing.T) {
	path := setupTestHubDB(t)
	insertUserMsg(t, path, "edit /tmp/test.md please")
	t.Setenv(hubDBPathEnvVar, path)
	got := collectDynamicPaths()
	if len(got) != 1 || got[0] != "/tmp/test.md" {
		t.Errorf("expected [/tmp/test.md], got %v", got)
	}
}

func TestCollectDynamicPaths_SlidingWindowCap(t *testing.T) {
	path := setupTestHubDB(t)
	// Insert 60 user msgs; only last 50 should be scanned. Insert
	// older msgs first with unique paths; newer last 50 have a
	// shared "newpath.md".
	for i := 0; i < 10; i++ {
		insertUserMsg(t, path, "/old/old"+string(rune('a'+i))+".md")
	}
	for i := 0; i < 50; i++ {
		insertUserMsg(t, path, "edit /tmp/newpath.md")
	}
	t.Setenv(hubDBPathEnvVar, path)
	got := collectDynamicPaths()
	for _, p := range got {
		if p == "/old/olda.md" {
			t.Errorf("oldest msg outside last-50 window should be excluded; got %v", got)
		}
	}
	found := false
	for _, p := range got {
		if p == "/tmp/newpath.md" {
			found = true
		}
	}
	if !found {
		t.Errorf("expected /tmp/newpath.md in result, got %v", got)
	}
}

func TestCollectDynamicPaths_OnlyUserAgent(t *testing.T) {
	path := setupTestHubDB(t)
	// Insert msg from non-user — should NOT be scanned.
	db, err := sql.Open("sqlite", path)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := db.Exec(
		`INSERT INTO messages(from_agent, content) VALUES('brian', 'edit /tmp/agent.md')`,
	); err != nil {
		t.Fatal(err)
	}
	db.Close()
	t.Setenv(hubDBPathEnvVar, path)
	got := collectDynamicPaths()
	if len(got) != 0 {
		t.Errorf("non-user msg should not contribute to dynamic paths, got %v", got)
	}
}

func TestMatchesUserArtifactPathWithDynamic_StaticPrecedence(t *testing.T) {
	home, _ := os.UserHomeDir()
	staticHit := filepath.Join(home, "Documents", "x.md")
	if !MatchesUserArtifactPathWithDynamic(staticHit, nil) {
		t.Error("static include should win even with empty dynamic paths")
	}
}

func TestMatchesUserArtifactPathWithDynamic_DynamicMatch(t *testing.T) {
	dynamicPaths := []string{"/srv/site"}
	if !MatchesUserArtifactPathWithDynamic("/srv/site/foo.md", dynamicPaths) {
		t.Error("dynamic include should match sub-path")
	}
}

func TestMatchesUserArtifactPathWithDynamic_SkipPrecedenceOverDynamic(t *testing.T) {
	dynamicPaths := []string{"/srv/site"}
	if MatchesUserArtifactPathWithDynamic("/srv/site/.git/config", dynamicPaths) {
		t.Error("SKIP must dominate dynamic include")
	}
}
