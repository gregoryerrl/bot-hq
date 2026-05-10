package gemma

import (
	"fmt"
	"log"
	"strings"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

func (g *Gemma) handleMentionDirective(msg protocol.Message, content string) {
	directive := stripEmmaMention(content)
	if directive == "" {
		log.Printf("emma: mention-directive from %s had empty body after @emma strip; dropping", msg.FromAgent)
		return
	}

	// T-1.5 R53 fix: require explicit `rule:` literal-prefix for rule-class
	// operations. Anything else is conversation-class (drop without rule
	// promotion). Eliminates structural cascade-mitigation overhead.
	if !hasRulePrefix(directive) {
		summary := directive
		if len(summary) > 80 {
			summary = summary[:80] + "..."
		}
		log.Printf("emma: mention from %s lacks `rule:` prefix; conversation-class drop (T-1.5 R53): %s", msg.FromAgent, summary)
		return
	}

	// Truncate for log + ack readability
	summary := directive
	if len(summary) > 80 {
		summary = summary[:80] + "..."
	}

	// Detect explicit purge-command (must come before generic rule-strip
	// since "rule: clear" matches both clear-pattern and rule-prefix).
	if isClearCommand(directive) {
		removed, err := ClearCustomRules()
		if err != nil {
			log.Printf("emma: ClearCustomRules failed for %s: %v", msg.FromAgent, err)
			g.db.InsertMessage(protocol.Message{
				FromAgent: agentID,
				Type:      protocol.MsgError,
				Content:   fmt.Sprintf("emma|custom-rules-clear-failed|from:%s|err:%v", msg.FromAgent, err),
			})
			return
		}
		log.Printf("emma: cleared %d custom rules at request of %s", removed, msg.FromAgent)
		g.db.InsertMessage(protocol.Message{
			FromAgent: agentID,
			Type:      protocol.MsgResult,
			Content:   fmt.Sprintf("emma|custom-rules-cleared|from:%s|removed:%d", msg.FromAgent, removed),
		})
		return
	}

	// Strip `rule:` prefix before persisting the rule body
	ruleBody := stripRulePrefix(directive)
	if ruleBody == "" {
		log.Printf("emma: mention from %s had empty body after `rule:` prefix strip; dropping", msg.FromAgent)
		return
	}

	rules, rejected, err := AppendCustomRule(ruleBody)
	if err != nil {
		log.Printf("emma: AppendCustomRule failed for %s: %v", msg.FromAgent, err)
		g.db.InsertMessage(protocol.Message{
			FromAgent: agentID,
			Type:      protocol.MsgError,
			Content:   fmt.Sprintf("emma|custom-rule-add-failed|from:%s|err:%v", msg.FromAgent, err),
		})
		return
	}
	if rejected != "" {
		log.Printf("emma: rejected directive from %s: %s", msg.FromAgent, rejected)
		g.db.InsertMessage(protocol.Message{
			FromAgent: agentID,
			Type:      protocol.MsgError,
			Content:   fmt.Sprintf("emma|custom-rule-rejected|from:%s|reason:%s", msg.FromAgent, rejected),
		})
		return
	}

	log.Printf("emma: custom rule added from %s (now %d active): %s", msg.FromAgent, len(rules), summary)
	g.db.InsertMessage(protocol.Message{
		FromAgent: agentID,
		Type:      protocol.MsgResult,
		Content:   fmt.Sprintf("emma|custom-rule-added|from:%s|active-count:%d|directive:%s|apply-at:next-enforcement-tick", msg.FromAgent, len(rules), summary),
	})
}

// stripEmmaMention removes the leading @emma token from content
// (case-insensitive) and trims surrounding whitespace, returning the
// directive body. Helper for handleMentionDirective.
func stripEmmaMention(content string) string {
	trimmed := strings.TrimSpace(content)
	// Find @emma (case-insensitive) anywhere — typically prefix.
	lower := strings.ToLower(trimmed)
	idx := strings.Index(lower, "@emma")
	if idx == -1 {
		return trimmed
	}
	// Skip past @emma + any trailing whitespace/punctuation.
	rest := trimmed[idx+len("@emma"):]
	// Allow common separators after @emma (space, colon, comma).
	for len(rest) > 0 && (rest[0] == ' ' || rest[0] == '\t' || rest[0] == ':' || rest[0] == ',') {
		rest = rest[1:]
	}
	return strings.TrimSpace(rest)
}

// isClearCommand reports whether directive is the explicit purge-
// command "rule: clear" (case-insensitive, with optional whitespace).
func isClearCommand(directive string) bool {
	lower := strings.ToLower(strings.TrimSpace(directive))
	return lower == "rule: clear" || lower == "rule:clear" ||
		lower == "clear rules" || lower == "clear custom rules"
}

// hasRulePrefix reports whether directive starts with the literal `rule:`
// prefix (case-insensitive, with optional surrounding whitespace). Per
// Phase T T-1.5 R53 EFFICIENCY-DRIVEN-DESIGN parser fix: only directives
// with this prefix are eligible for custom-rule promotion (or purge-via-
// `rule: clear`); everything else is conversation-class drop.
//
// Pre-fix cascade-bug: every @emma broadcast-mention body (any content
// <500 chars) auto-promoted as candidate-rule. Agents emitted >500-char
// hub_send to bypass via cap-rejection — structural overhead per R53.
// Fix per phase-t.md v5 T-1.5: require explicit `rule:` prefix.
//
// Also accepts the legacy "clear rules" / "clear custom rules" forms
// for backward-compat with existing isClearCommand patterns.
func hasRulePrefix(directive string) bool {
	lower := strings.ToLower(strings.TrimSpace(directive))
	if strings.HasPrefix(lower, "rule:") {
		return true
	}
	// Backward-compat: "clear rules" / "clear custom rules" route to
	// isClearCommand without explicit `rule:` prefix.
	if lower == "clear rules" || lower == "clear custom rules" {
		return true
	}
	return false
}

// stripRulePrefix removes the leading `rule:` prefix (case-insensitive)
// from directive + trims surrounding whitespace, returning the rule body.
// Helper for handleMentionDirective post-T-1.5 R53 parser fix.
func stripRulePrefix(directive string) string {
	trimmed := strings.TrimSpace(directive)
	lower := strings.ToLower(trimmed)
	if !strings.HasPrefix(lower, "rule:") {
		return trimmed
	}
	rest := trimmed[len("rule:"):]
	return strings.TrimSpace(rest)
}
