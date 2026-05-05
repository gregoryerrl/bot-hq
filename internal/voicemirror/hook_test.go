package voicemirror

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// runHookWith invokes RunHook with a JSON input + isolated env per
// R39 TEST-ISOLATION: tmp logPath + scoped env vars (t.Setenv reverts
// post-test).
func runHookWith(t *testing.T, input string, agentID, logPath string) int {
	t.Helper()
	if logPath != "" {
		t.Setenv(logPathEnvVar, logPath)
	}
	if agentID != "" {
		t.Setenv(agentIDEnvVar, agentID)
	}
	return RunHook(strings.NewReader(input), &bytes.Buffer{})
}

func mkInput(t *testing.T, toolName string, toolInput map[string]any) string {
	t.Helper()
	type i struct {
		ToolName  string         `json:"tool_name"`
		ToolInput map[string]any `json:"tool_input"`
	}
	b, err := json.Marshal(i{ToolName: toolName, ToolInput: toolInput})
	if err != nil {
		t.Fatalf("marshal input: %v", err)
	}
	return string(b)
}

func readFile(t *testing.T, p string) string {
	t.Helper()
	b, err := os.ReadFile(p)
	if err != nil {
		return ""
	}
	return string(b)
}

func TestNonWriteToolAllowed(t *testing.T) {
	in := `{"tool_name":"Bash","tool_input":{"command":"ls"}}`
	if got := runHookWith(t, in, "brian", ""); got != ExitAllow {
		t.Errorf("non-Write tool: expected ExitAllow=%d, got %d", ExitAllow, got)
	}
}

func TestBadJsonAllowed(t *testing.T) {
	if got := runHookWith(t, "not json", "brian", ""); got != ExitAllow {
		t.Errorf("bad json should allow defensively; got %d", got)
	}
}

func TestEmptyFilePathAllowed(t *testing.T) {
	in := mkInput(t, "Write", map[string]any{"content": "x"})
	if got := runHookWith(t, in, "brian", ""); got != ExitAllow {
		t.Errorf("empty file_path should allow; got %d", got)
	}
}

func TestUserDocumentAreaWriteLogged(t *testing.T) {
	home, _ := os.UserHomeDir()
	target := filepath.Join(home, "Documents", "main.json")
	logFile := filepath.Join(t.TempDir(), "log.md")
	in := mkInput(t, "Write", map[string]any{"file_path": target, "content": "user data"})
	if got := runHookWith(t, in, "brian", logFile); got != ExitAllow {
		t.Errorf("expected ExitAllow, got %d", got)
	}
	body := readFile(t, logFile)
	if !strings.Contains(body, target) {
		t.Errorf("log missing path %q; got: %s", target, body)
	}
	if !strings.Contains(body, "brian") {
		t.Errorf("log missing agent-id; got: %s", body)
	}
}

func TestUserDesktopAreaWriteLogged(t *testing.T) {
	home, _ := os.UserHomeDir()
	target := filepath.Join(home, "Desktop", "note.txt")
	logFile := filepath.Join(t.TempDir(), "log.md")
	in := mkInput(t, "Write", map[string]any{"file_path": target, "content": "x"})
	runHookWith(t, in, "rain", logFile)
	body := readFile(t, logFile)
	if !strings.Contains(body, target) {
		t.Errorf("Desktop write should match INCLUDE; got: %s", body)
	}
}

func TestProjectClaudeMdLogged(t *testing.T) {
	target := "/Users/u/Projects/myrepo/CLAUDE.md"
	logFile := filepath.Join(t.TempDir(), "log.md")
	in := mkInput(t, "Write", map[string]any{"file_path": target, "content": "user instructions"})
	runHookWith(t, in, "brian", logFile)
	body := readFile(t, logFile)
	if !strings.Contains(body, target) {
		t.Errorf("CLAUDE.md should match INCLUDE; got: %s", body)
	}
}

func TestProjectReadmeLogged(t *testing.T) {
	target := "/Users/u/Projects/myrepo/README.md"
	logFile := filepath.Join(t.TempDir(), "log.md")
	in := mkInput(t, "Write", map[string]any{"file_path": target, "content": "doc"})
	runHookWith(t, in, "brian", logFile)
	body := readFile(t, logFile)
	if !strings.Contains(body, target) {
		t.Errorf("README.md should match INCLUDE; got: %s", body)
	}
}

func TestProjectsPlansLogged(t *testing.T) {
	home, _ := os.UserHomeDir()
	target := filepath.Join(home, ".bot-hq", "projects", "myproj", "plans", "p.md")
	logFile := filepath.Join(t.TempDir(), "log.md")
	in := mkInput(t, "Write", map[string]any{"file_path": target, "content": "plan"})
	runHookWith(t, in, "brian", logFile)
	body := readFile(t, logFile)
	if !strings.Contains(body, target) {
		t.Errorf("projects/plans should match INCLUDE; got: %s", body)
	}
}

func TestProjectsEodLogged(t *testing.T) {
	home, _ := os.UserHomeDir()
	target := filepath.Join(home, ".bot-hq", "projects", "myproj", "eod", "report.md")
	logFile := filepath.Join(t.TempDir(), "log.md")
	in := mkInput(t, "Write", map[string]any{"file_path": target, "content": "eod"})
	runHookWith(t, in, "brian", logFile)
	body := readFile(t, logFile)
	if !strings.Contains(body, target) {
		t.Errorf("projects/eod should match INCLUDE; got: %s", body)
	}
}

func TestProjectsClipsLogged(t *testing.T) {
	home, _ := os.UserHomeDir()
	target := filepath.Join(home, ".bot-hq", "projects", "myproj", "clips", "c.md")
	logFile := filepath.Join(t.TempDir(), "log.md")
	in := mkInput(t, "Write", map[string]any{"file_path": target, "content": "clip"})
	runHookWith(t, in, "brian", logFile)
	body := readFile(t, logFile)
	if !strings.Contains(body, target) {
		t.Errorf("projects/clips should match INCLUDE; got: %s", body)
	}
}

func TestMemoryPathSkipped(t *testing.T) {
	target := "/Users/u/.claude/projects/foo/memory/note.md"
	logFile := filepath.Join(t.TempDir(), "log.md")
	in := mkInput(t, "Write", map[string]any{"file_path": target, "content": "x"})
	runHookWith(t, in, "brian", logFile)
	body := readFile(t, logFile)
	if strings.Contains(body, target) {
		t.Errorf("memory path should be SKIP-listed; log contains entry: %s", body)
	}
}

func TestProjectsOtherSubclassNotLogged(t *testing.T) {
	home, _ := os.UserHomeDir()
	target := filepath.Join(home, ".bot-hq", "projects", "myproj", "tmp", "scratch.md")
	logFile := filepath.Join(t.TempDir(), "log.md")
	in := mkInput(t, "Write", map[string]any{"file_path": target, "content": "x"})
	runHookWith(t, in, "brian", logFile)
	body := readFile(t, logFile)
	if strings.Contains(body, target) {
		t.Errorf("projects/<other-class> not in INCLUDE set; should not log: %s", body)
	}
}

func TestInRepoSourceNotLogged(t *testing.T) {
	target := "/Users/u/Projects/myrepo/internal/foo.go"
	logFile := filepath.Join(t.TempDir(), "log.md")
	in := mkInput(t, "Write", map[string]any{"file_path": target, "content": "code"})
	runHookWith(t, in, "brian", logFile)
	body := readFile(t, logFile)
	if strings.Contains(body, target) {
		t.Errorf("in-repo Go source not in INCLUDE; should not log: %s", body)
	}
}

func TestNodeModulesSkipped(t *testing.T) {
	target := "/Users/u/Projects/myrepo/node_modules/foo/CLAUDE.md"
	logFile := filepath.Join(t.TempDir(), "log.md")
	in := mkInput(t, "Write", map[string]any{"file_path": target, "content": "x"})
	runHookWith(t, in, "brian", logFile)
	body := readFile(t, logFile)
	if strings.Contains(body, target) {
		t.Errorf("node_modules CLAUDE.md should SKIP via skip-pattern; log: %s", body)
	}
}

func TestSnippetTruncated(t *testing.T) {
	home, _ := os.UserHomeDir()
	target := filepath.Join(home, "Documents", "big.md")
	logFile := filepath.Join(t.TempDir(), "log.md")
	longContent := strings.Repeat("x", 500)
	in := mkInput(t, "Write", map[string]any{"file_path": target, "content": longContent})
	runHookWith(t, in, "brian", logFile)
	body := readFile(t, logFile)
	if !strings.Contains(body, "…") {
		t.Errorf("expected snippet truncation marker '…' in log; got: %s", body)
	}
}

func TestMatchesUserArtifactPathExported(t *testing.T) {
	home, _ := os.UserHomeDir()
	cases := []struct {
		path string
		want bool
	}{
		{filepath.Join(home, "Documents", "x.md"), true},
		{filepath.Join(home, "Desktop", "y.txt"), true},
		{filepath.Join(home, ".bot-hq", "projects", "p", "plans", "doc.md"), true},
		{filepath.Join(home, ".bot-hq", "projects", "p", "memory", "anchor.md"), false},
		{"/Users/u/Projects/myrepo/CLAUDE.md", true},
		{"/Users/u/Projects/myrepo/internal/foo.go", false},
		{"/Users/u/Projects/myrepo/node_modules/x/CLAUDE.md", false},
	}
	for _, c := range cases {
		t.Run(c.path, func(t *testing.T) {
			if got := MatchesUserArtifactPath(c.path); got != c.want {
				t.Errorf("MatchesUserArtifactPath(%q) = %v; want %v", c.path, got, c.want)
			}
		})
	}
}
