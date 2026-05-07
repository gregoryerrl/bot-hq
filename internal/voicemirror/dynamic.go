// Phase N v2 OQ-1b graduation (Phase-R-followup): dynamic path
// extraction from recent user-msg content. Static include-set stays
// primary; dynamic extracts append additive include-paths from the
// last N user messages in hub.db.
//
// Hook stays alert-only; any DB / regex / parse error falls through
// to static-only with ExitAllow per fail-open semantics (Rain
// BRAIN-2nd Refine R1).
//
// Documented limitations:
//   - extension-less files (Makefile/Dockerfile) miss; regex requires
//     `.\w{1,8}` extension to suppress prose false-positives.
//   - paths with spaces miss (e.g., `~/Documents/My Folder/x.md`);
//     quoted-path detection is scope-creep. Accepted miss-rate.
//   - relative paths reject post-normalize; only absolute paths after
//     `~` expansion are user-voice-class.
package voicemirror

import (
	"database/sql"
	"os"
	"path/filepath"
	"regexp"
	"strings"

	_ "modernc.org/sqlite"
)

const (
	hubDBPathEnvVar      = "BOT_HQ_HUB_DB"
	dynamicMessageWindow = 50
)

// regexpUserPaths captures path-shape tokens with required file
// extension `.\w{1,8}\b`. Anchored to `~` / `/` / `$HOME` start.
var regexpUserPaths = regexp.MustCompile(`(?:\$HOME|~|/)[\w./_+-]+\.\w{1,8}\b`)

// extractPathsFromContent runs the user-path regex over a single
// content string and returns normalized absolute paths. Filters:
// reject relative-after-normalize; expand `~` / `$HOME` to user home.
func extractPathsFromContent(content string) []string {
	matches := regexpUserPaths.FindAllString(content, -1)
	if len(matches) == 0 {
		return nil
	}
	out := make([]string, 0, len(matches))
	seen := map[string]struct{}{}
	for _, m := range matches {
		p := expandHome(m)
		if !filepath.IsAbs(p) {
			continue
		}
		p = filepath.Clean(p)
		if _, dup := seen[p]; dup {
			continue
		}
		seen[p] = struct{}{}
		out = append(out, p)
	}
	return out
}

// expandHome replaces leading `~` or `$HOME` token with the user's
// home directory. Returns input unchanged if home cannot be resolved.
func expandHome(p string) string {
	home, err := os.UserHomeDir()
	if err != nil || home == "" {
		return p
	}
	if strings.HasPrefix(p, "~/") {
		return filepath.Join(home, p[2:])
	}
	if p == "~" {
		return home
	}
	if strings.HasPrefix(p, "$HOME/") {
		return filepath.Join(home, p[6:])
	}
	if p == "$HOME" {
		return home
	}
	return p
}

// openHubDBReadOnly opens hub.db in read-only WAL mode for hook-side
// queries. Path defaults to ~/.bot-hq/hub.db; BOT_HQ_HUB_DB env-var
// overrides for test-isolation per R39 TEST-ISOLATION.
func openHubDBReadOnly() (*sql.DB, error) {
	path := os.Getenv(hubDBPathEnvVar)
	if path == "" {
		home, _ := os.UserHomeDir()
		path = filepath.Join(home, ".bot-hq", "hub.db")
	}
	dsn := "file:" + path + "?mode=ro&_journal_mode=WAL"
	db, err := sql.Open("sqlite", dsn)
	if err != nil {
		return nil, err
	}
	if err := db.Ping(); err != nil {
		_ = db.Close()
		return nil, err
	}
	return db, nil
}

// recentUserMessages reads up to limit most-recent user-authored
// messages from the hub.db messages table. Returns content strings
// in DESC msg-id order (most recent first).
func recentUserMessages(db *sql.DB, limit int) ([]string, error) {
	rows, err := db.Query(
		`SELECT content FROM messages WHERE from_agent='user' ORDER BY id DESC LIMIT ?`,
		limit,
	)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []string
	for rows.Next() {
		var c string
		if err := rows.Scan(&c); err != nil {
			return nil, err
		}
		out = append(out, c)
	}
	return out, rows.Err()
}

// collectDynamicPaths reads recent user msgs + extracts user-path
// tokens. Returns deduped slice of normalized absolute paths.
// Fail-open: any DB error returns nil (caller falls through).
func collectDynamicPaths() []string {
	db, err := openHubDBReadOnly()
	if err != nil {
		return nil
	}
	defer db.Close()
	msgs, err := recentUserMessages(db, dynamicMessageWindow)
	if err != nil {
		return nil
	}
	seen := map[string]struct{}{}
	var out []string
	for _, m := range msgs {
		for _, p := range extractPathsFromContent(m) {
			if _, dup := seen[p]; dup {
				continue
			}
			seen[p] = struct{}{}
			out = append(out, p)
		}
	}
	return out
}

// MatchesDynamicInclude reports whether path equals or is a sub-path
// of any user-mentioned dynamic include. Sub-path check is prefix-
// with-separator (no false-match on partial-name overlap).
func MatchesDynamicInclude(path string, dynamicPaths []string) bool {
	for _, dp := range dynamicPaths {
		if path == dp {
			return true
		}
		if strings.HasPrefix(path, dp+string(filepath.Separator)) {
			return true
		}
	}
	return false
}
