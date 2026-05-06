// Package projectctx resolves the active bot-hq project for the current
// process — used by agent-init paths, the SessionStart hook, and the
// `bot-hq context-switch` CLI subcommand to pick a single canonical
// project for rule resolution + bootstrap loading.
//
// Resolution order (per design-spike §2.4):
//
//  1. BOT_HQ_PROJECT env var — authoritative if non-empty.
//  2. cwd-inference — walk up from $PWD looking for a .bot-hq/projects/<name>.yaml
//     or a .git ancestor whose `git remote get-url origin` derives a project
//     name with a matching ~/.bot-hq/projects/<name>.yaml file.
//  3. Default — "bot-hq" (Brian's home turf).
//
// Both signals are advisory: callers may override (e.g., explicit CLI flag).
package projectctx

import (
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/gregoryerrl/bot-hq/internal/projects"
)

// EnvVar is the env-var consulted first when resolving project context.
const EnvVar = "BOT_HQ_PROJECT"

// DefaultProject is the fallback when no other signal yields a project.
const DefaultProject = "bot-hq"

// Detect returns the active project for the current process. Never errors —
// falls through to DefaultProject if all signals fail.
func Detect() string {
	if p := strings.TrimSpace(os.Getenv(EnvVar)); p != "" {
		return p
	}
	if p := inferFromCwd(); p != "" {
		return p
	}
	return DefaultProject
}

// DetectFrom is the test-friendly variant: takes explicit cwd + canonRoot
// instead of reading process state. Same resolution order.
func DetectFrom(env, cwd, canonRoot string) string {
	if p := strings.TrimSpace(env); p != "" {
		return p
	}
	if p := inferFromDir(cwd, canonRoot); p != "" {
		return p
	}
	return DefaultProject
}

// inferFromCwd is the production cwd-inference: reads $PWD + ~/.bot-hq.
func inferFromCwd() string {
	cwd, err := os.Getwd()
	if err != nil {
		return ""
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return ""
	}
	canonRoot := filepath.Join(home, ".bot-hq")
	return inferFromDir(cwd, canonRoot)
}

// inferFromDir walks dir upward looking for a .git directory, then derives
// a project-name from its origin remote and verifies a matching
// canonRoot/projects/<name>.yaml exists. Returns "" on any failure.
//
// Walks at most 20 levels up to bound work.
func inferFromDir(dir, canonRoot string) string {
	if dir == "" {
		return ""
	}
	abs, err := filepath.Abs(dir)
	if err != nil {
		return ""
	}
	for i := 0; i < 20; i++ {
		gitDir := filepath.Join(abs, ".git")
		if info, err := os.Stat(gitDir); err == nil && (info.IsDir() || info.Mode().IsRegular()) {
			if name := projectFromGitDir(abs); name != "" {
				yamlPath := filepath.Join(canonRoot, "projects", name+".yaml")
				if _, err := os.Stat(yamlPath); err == nil {
					return name
				}
			}
			// Found .git but couldn't resolve to a registered project; stop walking.
			return ""
		}
		parent := filepath.Dir(abs)
		if parent == abs {
			return ""
		}
		abs = parent
	}
	return ""
}

// projectFromGitDir runs `git -C dir remote get-url origin` and derives the
// project name. Returns "" on any failure.
func projectFromGitDir(dir string) string {
	out, err := exec.Command("git", "-C", dir, "remote", "get-url", "origin").Output()
	if err != nil {
		return ""
	}
	url := strings.TrimSpace(string(out))
	return projects.DeriveProjectName(url)
}
