// Package bootstrap implements per-project session-bootstrap snapshot
// read + write. Bootstraps live at ~/.bot-hq/projects/<p>/bootstrap.md
// (one per project). They carry a frontmatter block + free-form summary.
//
// Write triggers (per design-spike §2.3):
//
//   - graceful: SessionEnd hook fires
//   - defensive: every 25 hub-msgs OR 10 minutes (whichever first)
//   - precompact: PreCompact hook fires
//   - stale: read-time marker when bootstrap is >24h old without refresh
//
// Atomicity: writes go to bootstrap.md.tmp first, then rename — readers
// never observe torn state.
//
// The orchestrator (cmd/bot-hq/main.go runHub) is the canonical writer;
// agents read via /api/session-open. This package supplies both halves.
package bootstrap

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"gopkg.in/yaml.v3"
)

// Frontmatter is the YAML head-block of bootstrap.md. All fields optional;
// new keys allowed for forward-compat.
type Frontmatter struct {
	LastSessionID      string    `yaml:"last_session_id,omitempty" json:"last_session_id,omitempty"`
	LastSessionCloseAt time.Time `yaml:"last_session_close_at,omitempty" json:"last_session_close_at,omitempty"`
	PhaseOrMilestone   string    `yaml:"phase_or_milestone,omitempty" json:"phase_or_milestone,omitempty"`
	KeyState           string    `yaml:"key_state,omitempty" json:"key_state,omitempty"`
	WriteTrigger       string    `yaml:"write_trigger,omitempty" json:"write_trigger,omitempty"` // graceful | defensive | precompact | stale
	LastNPeerCoord     []string  `yaml:"last_n_peer_coord,omitempty" json:"last_n_peer_coord,omitempty"`
}

// Bootstrap bundles the parsed frontmatter + body. Write/Read round-trip.
type Bootstrap struct {
	Frontmatter Frontmatter
	Body        string
}

// Path returns the on-disk path for project p's bootstrap.md.
func Path(canonRoot, project string) string {
	return filepath.Join(canonRoot, "projects", project, "bootstrap.md")
}

// Write atomically writes b to the bootstrap path for project. Creates the
// project dir if missing. Uses temp-file + rename per design-spike §2.3.
func Write(canonRoot, project string, b Bootstrap) error {
	if project == "" {
		return errors.New("project required")
	}
	dir := filepath.Join(canonRoot, "projects", project)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return fmt.Errorf("mkdir %s: %w", dir, err)
	}
	final := Path(canonRoot, project)
	tmp := final + ".tmp"

	content, err := encode(b)
	if err != nil {
		return err
	}
	if err := os.WriteFile(tmp, []byte(content), 0o644); err != nil {
		return fmt.Errorf("write tmp %s: %w", tmp, err)
	}
	if err := os.Rename(tmp, final); err != nil {
		_ = os.Remove(tmp)
		return fmt.Errorf("rename %s -> %s: %w", tmp, final, err)
	}
	return nil
}

// Read parses the bootstrap.md for project. Returns (nil, nil) if absent —
// caller treats this as "no prior session" rather than an error.
//
// Stale detection: if Frontmatter.LastSessionCloseAt is older than 24h
// AND WriteTrigger is not already "stale", the returned struct's
// WriteTrigger is overwritten to "stale" so the consumer (session-open
// formatter) can flag risk to the agent.
func Read(canonRoot, project string) (*Bootstrap, error) {
	p := Path(canonRoot, project)
	data, err := os.ReadFile(p)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return nil, nil
		}
		return nil, err
	}
	b, err := Decode(string(data))
	if err != nil {
		return nil, err
	}
	if !b.Frontmatter.LastSessionCloseAt.IsZero() &&
		time.Since(b.Frontmatter.LastSessionCloseAt) > 24*time.Hour &&
		b.Frontmatter.WriteTrigger != "stale" {
		b.Frontmatter.WriteTrigger = "stale"
	}
	return b, nil
}

// encode serializes b to the on-disk markdown form: frontmatter delimited
// by `---` lines, then the free-form body.
func encode(b Bootstrap) (string, error) {
	var buf strings.Builder
	buf.WriteString("---\n")
	fmBytes, err := yaml.Marshal(b.Frontmatter)
	if err != nil {
		return "", fmt.Errorf("marshal frontmatter: %w", err)
	}
	buf.Write(fmBytes)
	buf.WriteString("---\n\n")
	buf.WriteString(b.Body)
	if !strings.HasSuffix(b.Body, "\n") {
		buf.WriteString("\n")
	}
	return buf.String(), nil
}

// Decode parses raw on-disk content into Bootstrap. Tolerates missing
// frontmatter (treats whole file as body) for forward-compat with files
// authored by hand.
func Decode(raw string) (*Bootstrap, error) {
	b := &Bootstrap{}
	if !strings.HasPrefix(raw, "---\n") && !strings.HasPrefix(raw, "---\r\n") {
		b.Body = raw
		return b, nil
	}
	// Strip leading "---\n" (or "---\r\n").
	rest := raw[4:]
	if strings.HasPrefix(raw, "---\r\n") {
		rest = raw[5:]
	}
	end := strings.Index(rest, "\n---")
	if end < 0 {
		// Malformed — no closing delimiter. Treat whole file as body.
		b.Body = raw
		return b, nil
	}
	fm := rest[:end]
	body := rest[end+4:] // skip "\n---"
	body = strings.TrimPrefix(body, "\n")
	body = strings.TrimPrefix(body, "\r\n")

	if err := yaml.Unmarshal([]byte(fm), &b.Frontmatter); err != nil {
		return nil, fmt.Errorf("parse frontmatter: %w", err)
	}
	b.Body = body
	return b, nil
}
