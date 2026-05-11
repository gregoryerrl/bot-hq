// Package protocol — mention.go: @<agent> mention parser for
// broadcast-mention-based routing. Phase-S-followup-2 F2-1.
//
// Background: Phase S S-4 dropped the hub_send `to:` parameter; all
// new messages broadcast (ToAgent==""). Mention-based targeting via
// `@<agent>` content-prefix replaces PM-class targeting at the agent
// self-filter layer. Per PhaseSv1AudienceClassLoadBearing rule-text,
// canonical format: `@brian` / `@rain` / `@emma` / `@<coder-id>`.
//
// This parser exposes the regex as code so emma's gemma.go:549
// broadcast-filter can route mention-targeted messages without
// requiring ToAgent to be set. M4 from Phase-S-followup-2 audit.
package protocol

import "regexp"

// mentionPattern matches `@<agent>` mentions in message content.
// Match preconditions: preceded by start-of-string or whitespace
// (boundary), followed by word-boundary. Case-insensitive on the
// agent token. Coder IDs match `coder-<hex>` form.
var mentionPattern = regexp.MustCompile(`(?i)(?:^|\s)@(brian|rain|emma|coder-[a-f0-9]+)\b`)

// emmaBareNamePattern accepts natural-language addressing of Emma at
// the start of a message: bare "emma", optional greeting + "emma".
// Only consulted for main-hub messages (session_id=="") to avoid
// over-routing in busy session timelines where "the emma binary…" etc.
// are content references, not addresses.
var emmaBareNamePattern = regexp.MustCompile(`(?i)^\s*(?:(?:hi|hey|yo|hello|sup)\b[\s,!.]*)?emma\b`)

// MentionsAgent reports whether content addresses the given agent ID
// via @<agent> mention. agentID is matched case-insensitively;
// coder-* IDs match exactly (full coder-<hex> string).
func MentionsAgent(content, agentID string) bool {
	if agentID == "" {
		return false
	}
	matches := mentionPattern.FindAllStringSubmatch(content, -1)
	for _, m := range matches {
		if len(m) >= 2 && equalFold(m[1], agentID) {
			return true
		}
	}
	return false
}

// MentionsAgentLenient is MentionsAgent with one carve-out: for Emma
// in main-hub messages (sessionID==""), bare-name addressing at the
// start of the message also counts (e.g. "hi emma", "emma?"). Per-
// session messages stay strict to avoid over-routing on content
// references. Z-5a.
func MentionsAgentLenient(content, agentID, sessionID string) bool {
	if MentionsAgent(content, agentID) {
		return true
	}
	if sessionID == "" && equalFold(agentID, "emma") {
		return emmaBareNamePattern.MatchString(content)
	}
	return false
}

// ExtractMentions returns all @<agent> mentions in content as a list
// of agent IDs (lower-cased; deduped in-order). Useful for routing
// + audit when a single message addresses multiple agents.
func ExtractMentions(content string) []string {
	matches := mentionPattern.FindAllStringSubmatch(content, -1)
	if len(matches) == 0 {
		return nil
	}
	seen := make(map[string]bool, len(matches))
	out := make([]string, 0, len(matches))
	for _, m := range matches {
		if len(m) < 2 {
			continue
		}
		id := toLower(m[1])
		if seen[id] {
			continue
		}
		seen[id] = true
		out = append(out, id)
	}
	return out
}

// equalFold compares two strings case-insensitively (ASCII).
// Avoids importing strings.EqualFold to keep this package light.
func equalFold(a, b string) bool {
	if len(a) != len(b) {
		return false
	}
	for i := 0; i < len(a); i++ {
		ca, cb := a[i], b[i]
		if ca >= 'A' && ca <= 'Z' {
			ca += 'a' - 'A'
		}
		if cb >= 'A' && cb <= 'Z' {
			cb += 'a' - 'A'
		}
		if ca != cb {
			return false
		}
	}
	return true
}

func toLower(s string) string {
	b := make([]byte, len(s))
	for i := 0; i < len(s); i++ {
		c := s[i]
		if c >= 'A' && c <= 'Z' {
			c += 'a' - 'A'
		}
		b[i] = c
	}
	return string(b)
}
