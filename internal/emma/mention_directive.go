package emma

import (
	"context"
	"fmt"
	"log"
	"strings"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

func (g *Emma) handleMentionDirective(msg protocol.Message, content string) {
	directive := stripEmmaMention(content)
	if directive == "" {
		log.Printf("emma: mention-directive from %s had empty body after @emma strip; dropping", msg.FromAgent)
		return
	}

	// Z-5g per vision.md: Emma is the global hub orchestrator, NOT a
	// rule-enforcer. Phase T R53's "rule:-or-drop" gate was correct
	// before Z-0 / Z-1 / Z-3 repositioned Emma as the user's conversation
	// partner for session-lifecycle + CL pointer + project brainstorm.
	// New routing:
	//
	//   - "@emma rule: ..."     → custom-rules.md operations (preserved)
	//   - "@emma <conversation>" → LLM-generated reply posted back to hub
	//                              in the same session_id (so a per-session
	//                              "ask emma" routes inside that session)
	//
	// The cascade-bug R53 was guarding against was "every mention body
	// auto-promoted to a rule" — that's still prevented because rule
	// promotion still requires the `rule:` literal prefix; conversation
	// replies don't touch custom-rules.md at all.
	if !hasRulePrefix(directive) {
		g.handleConversationMention(msg, directive)
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

// handleConversationMention replies to a non-rule-prefixed user mention
// using Emma's hub-orchestrator persona per vision.md (look at CL, point
// BRAIN-duo, manage session lifecycle, brainstorm projects). The reply
// is posted back to hub with the same session_id as the inbound message
// so per-session "ask emma" stays inside the session view, and main-hub
// chat stays on main hub.
//
// Z-5g. Concurrency-capped via SharedSem (same pool as TaskAnalyze).
func (g *Emma) handleConversationMention(msg protocol.Message, directive string) {
	summary := directive
	if len(summary) > 80 {
		summary = summary[:80] + "..."
	}
	log.Printf("emma: conversation-mention from %s (session=%q): %s", msg.FromAgent, msg.SessionID, summary)

	ctx, cancel := context.WithTimeout(context.Background(), 90*time.Second)
	defer cancel()

	// Bound concurrent generation against SharedSem (same pool that
	// TaskAnalyze + hub_spawn_gemma compete for).
	select {
	case SharedSem <- struct{}{}:
		defer func() { <-SharedSem }()
	case <-ctx.Done():
		log.Printf("emma: conversation reply timed out acquiring SharedSem for %s", msg.FromAgent)
		return
	}

	prompt := buildConversationPrompt(directive)
	reply, err := g.client.Generate(ctx, prompt)
	if err != nil {
		log.Printf("emma: conversation Generate failed for %s: %v", msg.FromAgent, err)
		g.db.InsertMessage(protocol.Message{
			FromAgent: agentID,
			ToAgent:   msg.FromAgent,
			Type:      protocol.MsgError,
			Content:   fmt.Sprintf("emma reply failed: %v", err),
			SessionID: msg.SessionID,
		})
		return
	}
	reply = strings.TrimSpace(reply)
	if reply == "" {
		reply = "(emma had no reply — try rephrasing)"
	}
	if _, err := g.db.InsertMessage(protocol.Message{
		FromAgent: agentID,
		// Z-5i: broadcast reply (was ToAgent: msg.FromAgent). Matches the
		// user-mockup "[main] emma: ..." with no arrow. The user posted
		// the question in the open hub; Emma's answer goes on the open
		// hub too.
		Type:      protocol.MsgUpdate,
		Content:   reply,
		SessionID: msg.SessionID,
	}); err != nil {
		log.Printf("emma: failed to post conversation reply for %s: %v", msg.FromAgent, err)
	}
}

// buildConversationPrompt frames Emma's hub-orchestrator persona per
// vision.md so the model knows what scope to answer in. Keeps prompt
// short for gemma4:e4b context budget; vision.md anchor is named
// (not pasted in full) so we don't blow context on every turn.
func buildConversationPrompt(userMessage string) string {
	return `You are Emma, bot-hq's global hub orchestrator (DeepSeek-V4-Pro per vision.md, currently running on gemma4:e4b).

Per vision.md your role is:
- Look at CL (the Context Library at ~/.bot-hq/) to ground answers
- Point BRAIN-duo (brian + rain) at relevant areas when the user wants work done
- Manage session lifecycle (open/close sessions)
- Brainstorm new project ideation with the user
- You do NOT participate in BRAIN-cycle; you do NOT hold state; you do NOT elevate

Be concise (under 4 sentences unless the user asked for detail). If the user wants action, suggest the bot-hq tool that achieves it (hub_session_open / hub_send / etc.) rather than describing what you would do.

User: ` + userMessage + `

Emma:`
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
