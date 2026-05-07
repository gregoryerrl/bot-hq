package voicemirror

import (
	"database/sql"
	"os"
	"path/filepath"
	"strings"
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
			t.Errorf("unquoted space-in-path should not match full path (documented limitation): got %v", got)
		}
	}
	// Documented behavior: bare regex may extract partial sub-path
	// after the space (e.g., "/x.md"); accepted limitation. Quoted
	// variants `"..."` / `'...'` admit spaces — see
	// TestExtractPathsFromContent_DoubleQuotedSpaceInPath.
}

func TestExtractPathsFromContent_DoubleQuotedSpaceInPath(t *testing.T) {
	home, _ := os.UserHomeDir()
	got := extractPathsFromContent(`edit "~/My Folder/x.md" please`)
	want := filepath.Join(home, "My Folder/x.md")
	found := false
	for _, p := range got {
		if p == want {
			found = true
		}
	}
	if !found {
		t.Errorf("expected double-quoted space-in-path to match %s, got %v", want, got)
	}
}

func TestExtractPathsFromContent_SingleQuotedSpaceInPath(t *testing.T) {
	home, _ := os.UserHomeDir()
	got := extractPathsFromContent("edit '~/My Folder/x.md' please")
	want := filepath.Join(home, "My Folder/x.md")
	found := false
	for _, p := range got {
		if p == want {
			found = true
		}
	}
	if !found {
		t.Errorf("expected single-quoted space-in-path to match %s, got %v", want, got)
	}
}

func TestExtractPathsFromContent_QuotedAbsolutePathWithSpace(t *testing.T) {
	got := extractPathsFromContent(`use "/srv/My Site/index.html" today`)
	want := "/srv/My Site/index.html"
	found := false
	for _, p := range got {
		if p == want {
			found = true
		}
	}
	if !found {
		t.Errorf("expected double-quoted absolute path with space to match %s, got %v", want, got)
	}
}

func TestExtractPathsFromContent_QuotedHOMEExpansion(t *testing.T) {
	home, _ := os.UserHomeDir()
	got := extractPathsFromContent(`open "$HOME/My Folder/file.md"`)
	want := filepath.Join(home, "My Folder/file.md")
	found := false
	for _, p := range got {
		if p == want {
			found = true
		}
	}
	if !found {
		t.Errorf("expected $HOME-prefixed quoted path to match %s, got %v", want, got)
	}
}

func TestExtractPathsFromContent_BackslashEscapedSpaceMisses(t *testing.T) {
	// Documented residual limitation: backslash-escaped spaces are
	// rare in user prose and not handled. Regex token rejects backslash
	// in path body via `[\w./_+-]+`.
	home, _ := os.UserHomeDir()
	got := extractPathsFromContent(`edit ~/My\ Folder/x.md please`)
	wantFull := filepath.Join(home, "My Folder/x.md")
	for _, p := range got {
		if p == wantFull {
			t.Errorf("backslash-escaped space should not match (documented limit): got %v", got)
		}
	}
}

func TestExtractPathsFromContent_UnclosedQuoteRejects(t *testing.T) {
	// Unclosed quote: regex requires closing quote, so no match.
	got := extractPathsFromContent(`edit "~/My Folder/x.md please`)
	for _, p := range got {
		if strings.Contains(p, "My Folder") {
			t.Errorf("unclosed-quote path should not match, got %v", got)
		}
	}
}

func TestExtractPathsFromContent_BareAndQuotedDedupe(t *testing.T) {
	got := extractPathsFromContent(`edit /tmp/a.md and "/tmp/a.md" again`)
	if len(got) != 1 || got[0] != "/tmp/a.md" {
		t.Errorf("expected single deduped /tmp/a.md, got %v", got)
	}
}

func TestExtractPathsFromContent_LiteralMakefile(t *testing.T) {
	home, _ := os.UserHomeDir()
	got := extractPathsFromContent("update ~/project/Makefile please")
	want := filepath.Join(home, "project/Makefile")
	found := false
	for _, p := range got {
		if p == want {
			found = true
		}
	}
	if !found {
		t.Errorf("expected literal-allowlist Makefile to match, got %v", got)
	}
}

func TestExtractPathsFromContent_LiteralDockerfile(t *testing.T) {
	got := extractPathsFromContent("rebuild /srv/app/Dockerfile in CI")
	want := "/srv/app/Dockerfile"
	found := false
	for _, p := range got {
		if p == want {
			found = true
		}
	}
	if !found {
		t.Errorf("expected literal-allowlist Dockerfile to match, got %v", got)
	}
}

func TestExtractPathsFromContent_LiteralRootLevel(t *testing.T) {
	got := extractPathsFromContent("the file at /Makefile is root-level")
	want := "/Makefile"
	found := false
	for _, p := range got {
		if p == want {
			found = true
		}
	}
	if !found {
		t.Errorf("expected root-level /Makefile to match, got %v", got)
	}
}

func TestExtractPathsFromContent_LiteralProseFalsePositiveSuppressed(t *testing.T) {
	// "the Makefile" mentioned generically (no leading `/` or `~/`)
	// must not trigger — allowlist anchors require path-shape.
	got := extractPathsFromContent("the Makefile in this project is broken")
	for _, p := range got {
		if strings.HasSuffix(p, "Makefile") {
			t.Errorf("prose mention should not false-positive, got %v", got)
		}
	}
}

func TestExtractPathsFromContent_LiteralCMakeListsTxt(t *testing.T) {
	got := extractPathsFromContent("regenerate /build/CMakeLists.txt now")
	want := "/build/CMakeLists.txt"
	found := false
	for _, p := range got {
		if p == want {
			found = true
		}
	}
	if !found {
		t.Errorf("expected CMakeLists.txt to match, got %v", got)
	}
}

func TestExtractPathsFromContent_LiteralDotEnv(t *testing.T) {
	home, _ := os.UserHomeDir()
	got := extractPathsFromContent("update ~/app/.env config")
	want := filepath.Join(home, "app/.env")
	found := false
	for _, p := range got {
		if p == want {
			found = true
		}
	}
	if !found {
		t.Errorf("expected .env to match, got %v", got)
	}
}

func TestExtractPathsFromContent_LiteralAllowlistFull(t *testing.T) {
	// Every allowlist entry should match in path-tail position.
	cases := []struct {
		content string
		want    string
	}{
		{"see /a/Makefile", "/a/Makefile"},
		{"see /a/Dockerfile", "/a/Dockerfile"},
		{"see /a/Procfile", "/a/Procfile"},
		{"see /a/Rakefile", "/a/Rakefile"},
		{"see /a/Gemfile", "/a/Gemfile"},
		{"see /a/CMakeLists.txt", "/a/CMakeLists.txt"},
		{"see /a/.env", "/a/.env"},
	}
	for _, tc := range cases {
		got := extractPathsFromContent(tc.content)
		found := false
		for _, p := range got {
			if p == tc.want {
				found = true
			}
		}
		if !found {
			t.Errorf("content=%q want=%s got=%v", tc.content, tc.want, got)
		}
	}
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
