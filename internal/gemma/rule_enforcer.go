// Package gemma — rule_enforcer.go: emma-as-rule-enforcer functional
// substrate per Phase-S-followup-1 F1-4 hybrid (β)+(γ) scope per
// Rain BRAIN-2nd msg 15985 (post user msg 15997 cost-class
// clarification "gemma 4 e4b has no cost, we run it locally" which
// invalidated the cost-class concern that previously favored (ε)
// scope-narrow).
//
// Resolves phase-s.md S-1b §169 spec-vs-gemma-§72-H-24-boundary
// structural contradiction via hybrid mechanism:
//   - (β) Pattern-match Go-side for 3 mechanical-class items
//     (preserves §72 boundary intact)
//   - (γ) LLM-judgment via local Client.Generate for 5 interpretive-
//     class items (authorized via §72 amendment carve-out — rule-
//     enforcement-as-emma-role is authorized interpretive class)
//
// Full coverage of all 8 scope-prior baseline items per phase-s.md
// §175 hybrid-rewrite. Matches user msg 15734 "emma will be the
// enforcer" + msg 15936 "NOTHING PENDING" + msg 15966 "REWRITE
// STRUCTURALLY UNFOLLOWABLE TASKS" full-enforcer reading.
//
// Speech-trigger: silent unless (i) `@emma` mention OR (ii) mechanical-
// class violation pattern-match-detected OR (iii) interpretive-class
// violation LLM-judgment-detected. Anti-drift clause: watch-confirm /
// peer-coord-ack class is NOT speech-trigger (prevents ~9 watch-
// confirm drift Phase S empirical msg 15910).
package gemma

import (
	"context"
	"fmt"
	"log"
	"regexp"
	"strings"
	"sync"
	"time"

	"github.com/gregoryerrl/bot-hq/internal/hub"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

const (
	// ruleEnforcerInterval is the cadence for the enforcement cycle.
	// Hybrid (β)+(γ) per Rain R-7 5-10 min lean: gemma4:e4b LLM-
	// judgment loop runs at this cadence on rolling 25-msg window.
	// Mechanical pattern-match runs same cadence (cheap; no separate
	// goroutine). 5 min balances real-time enforcement vs LLM-cycle
	// noise + bounds CPU local.
	ruleEnforcerInterval = 5 * time.Minute

	// ruleEnforcerBatchSize is the rolling-window message count
	// analyzed per enforcement cycle.
	ruleEnforcerBatchSize = 25

	// ruleEnforcerAgentID is the FromAgent value for emit-violations.
	// Same as gemma agentID — emma-the-role on gemma-the-process.
	ruleEnforcerAgentID = "emma"

	// dotParkingThreshold is the per-agent bare-`.`-emit count within
	// the dotParkingWindow that triggers a heartbeat-loop-antipattern
	// violation. Threshold 3 reflects Phase S empirical: bilateral
	// ack-loops typically 2-3 cycles before user intervention.
	dotParkingThreshold = 3
	dotParkingWindow    = 5 * time.Minute

	// shapeDisclosureCooldown prevents re-firing R37 violation on the
	// same agent rapidly; one violation per cooldown window.
	shapeDisclosureCooldown = 30 * time.Minute

	// snapGatingCooldown prevents re-firing SNAP violation rapidly.
	snapGatingCooldown = 30 * time.Minute
)

// dotParkingPattern matches a bare `.` MsgUpdate content (handshake
// terminator). Allows trailing whitespace.
var dotParkingPattern = regexp.MustCompile(`^\s*\.\s*$`)

// shapeDisclosureEstimatePattern matches an estimate-emit (e.g.,
// "~150-300 LOC", "estimate 500-1000", "est ~80-120"). Triggers
// shape-disclosure check.
var shapeDisclosureEstimatePattern = regexp.MustCompile(`(?i)\b(?:est(?:imate)?|envelope|loc[\s:]+\d|~\d+[-–]\d+\s*LOC)\b`)

// shapeDisclosureTagPattern matches a present R37 shape-category tag
// (sparse / dense / mechanical / foundation / mirror / core-impl /
// core\+tests / state-edit / rename-mechanical / rule-text-consts).
var shapeDisclosureTagPattern = regexp.MustCompile(`(?i)\b(?:sparse|dense|mechanical|foundation|mirror|core-impl|core\+tests|state-edit|rename-mechanical|rule-text-consts|estimate-shape|shape:)\b`)

// snapBlockPattern matches a SNAP-block: at minimum
// "Branches:" + "Agents:" + "Pending:" + "Next:" all present in
// the same message content.
var snapBlockPattern = regexp.MustCompile(`(?si)Branches:.*Agents:.*Pending:.*Next:`)

// phaseTransitionPattern matches phase-transition keywords that
// LEGITIMATELY warrant SNAP-block per R5 SNAP-GATING. Absence of
// these keywords + presence of SNAP-block = potential violation.
var phaseTransitionPattern = regexp.MustCompile(`(?i)\b(?:commit-land|commit-fire|push-fire|PR-open|halt-ack|session-close|phase-close|phase-open|BRAIN-final-seal|LANDED|PUSHED|SEAL|CLOSED-PUBLIC)\b`)

// ruleEnforcerLLMPrompt is the LLM-judgment prompt template for
// interpretive-class violation detection per (γ). Authorized via
// gemma canonicalEmmaBlock §72-amendment (rule-enforcement-as-emma-
// role is authorized interpretive carve-out).
const ruleEnforcerLLMPrompt = `You are Emma, bot-hq's rule-enforcer (special interpretive carve-out per H-24 + Phase-S-followup-1 §72-amendment).

Watch the recent hub messages below and identify any rule-violations from these 5 interpretive-class rules:

R-INT-1 NON-CONTINUATION-AFTER-USER-DIRECTIVE: agent stopped or asked clarifying question after user gave clear directive (e.g., "proceed", "smoke them all", "all of these done NOW") instead of executing.

R-INT-2 CROSS-TIMING-DEDUP MISUSE: agent quoted >2 contiguous lines of peer message inline OR did not use "[crossed in flight — see msg N]" terse-reference pattern when peer's recent message covered drafted content.

R-INT-3 HANDSHAKE-TERMINATOR MISUSE: agent emitted "." parking-class when peer's most-recent message had substantive new content (HANDSHAKE-ACK-BLIND-SPOT class).

R-INT-4 SCOPE-FORK-CONFIRMATION SKIPPED: user phrasing had fork-able scope (UNTIL/INCLUDING/JUST/etc. ambiguity OR push/commit/merge fork) but agent fired without surfacing interpretation pre-action via hub_send.

R-INT-5 FILESYSTEM-SIGNAL-CITE SKIPPED: agent made claim derived from filesystem-state (empty git diff / clean git status / file absence / wc-l zero) without naming the signal command + acknowledging interpretation-limitations.

For each violation, output ONE LINE in format:
VIOLATION: <rule-id> | msg: <msg-id> | <brief-cite-discriminator>

If no violations, output exactly:
NO VIOLATIONS

Default-deny on uncertain class — output NO VIOLATIONS rather than false-positive.

Recent hub messages:
%s

Output:`

// RuleEnforcer is the emma-as-rule-enforcer functional substrate.
// Spawned as a goroutine in Gemma.Start() lifecycle.
//
// Hybrid (β)+(γ) scope: mechanical pattern-match detection (3 items)
// + LLM-judgment via local gemma Client.Generate (5 items, no cost
// per user msg 15997). §72-amendment authorizes rule-enforcement
// interpretive carve-out for emma's own role.
type RuleEnforcer struct {
	db       *hub.DB
	client   *Client
	interval time.Duration

	mu      sync.Mutex
	running bool
	stopCh  chan struct{}

	// detectorState tracks per-rule cooldown timestamps to prevent
	// rapid re-firing on the same violation pattern.
	detectorMu        sync.Mutex
	dotParkingHistory map[string][]time.Time
	shapeLastFired    map[int64]time.Time
	snapLastFired     map[int64]time.Time

	// nowFunc returns the current time; injectable for tests.
	nowFunc func() time.Time

	// llmEnabled gates the LLM-judgment loop. Default true; tests can
	// disable to exercise pattern-match path only or to inject a mock
	// client without LLM dependency.
	llmEnabled bool
}

// NewRuleEnforcer constructs a RuleEnforcer with default cadence.
// client may be nil; LLM-judgment is skipped silently when client is
// nil (mechanical pattern-match still runs).
func NewRuleEnforcer(db *hub.DB, client *Client) *RuleEnforcer {
	return &RuleEnforcer{
		db:                db,
		client:            client,
		interval:          ruleEnforcerInterval,
		stopCh:            make(chan struct{}),
		dotParkingHistory: make(map[string][]time.Time),
		shapeLastFired:    make(map[int64]time.Time),
		snapLastFired:     make(map[int64]time.Time),
		nowFunc:           time.Now,
		llmEnabled:        true,
	}
}

// SetLLMEnabled toggles the LLM-judgment loop (test-only).
func (re *RuleEnforcer) SetLLMEnabled(enabled bool) {
	re.mu.Lock()
	defer re.mu.Unlock()
	re.llmEnabled = enabled
}

// SetInterval overrides the enforcement-cycle cadence (test-only).
func (re *RuleEnforcer) SetInterval(d time.Duration) {
	re.mu.Lock()
	defer re.mu.Unlock()
	re.interval = d
}

// SetNowFunc overrides the time source (test-only).
func (re *RuleEnforcer) SetNowFunc(fn func() time.Time) {
	re.detectorMu.Lock()
	defer re.detectorMu.Unlock()
	re.nowFunc = fn
}

// Start spawns the enforcement-loop goroutine.
func (re *RuleEnforcer) Start() error {
	re.mu.Lock()
	if re.running {
		re.mu.Unlock()
		return nil
	}
	re.running = true
	re.mu.Unlock()

	go re.enforcementLoop()
	return nil
}

// Stop halts the enforcement-loop goroutine.
func (re *RuleEnforcer) Stop() {
	re.mu.Lock()
	defer re.mu.Unlock()
	if !re.running {
		return
	}
	close(re.stopCh)
	re.running = false
}

// IsRunning reports whether the enforcement loop is active.
func (re *RuleEnforcer) IsRunning() bool {
	re.mu.Lock()
	defer re.mu.Unlock()
	return re.running
}

func (re *RuleEnforcer) enforcementLoop() {
	ticker := time.NewTicker(re.interval)
	defer ticker.Stop()
	for {
		select {
		case <-re.stopCh:
			return
		case <-ticker.C:
			re.RunEnforcementCycle()
		}
	}
}

// RunEnforcementCycle executes one enforcement pass: fetches recent
// hub messages, runs 3 mechanical pattern-match detectors + 5
// interpretive-class LLM-judgment detection (per (γ) §72-amendment
// carve-out), emits violations.
//
// Exported for test injection. Idempotent on no-violations input.
func (re *RuleEnforcer) RunEnforcementCycle() {
	msgs, err := re.db.GetRecentMessages(ruleEnforcerBatchSize)
	if err != nil || len(msgs) == 0 {
		return
	}

	// Pattern-match (mechanical class — fast, no LLM call)
	re.DetectDotParking(msgs)
	re.DetectShapeDisclosureSkipped(msgs)
	re.DetectSnapGatingViolation(msgs)

	// LLM-judgment (interpretive class — gemma4:e4b local-no-cost)
	re.mu.Lock()
	llmOn := re.llmEnabled
	re.mu.Unlock()
	if llmOn && re.client != nil {
		re.detectInterpretiveViolations(msgs)
	}
}

// detectInterpretiveViolations builds the LLM-judgment prompt + calls
// gemma Client.Generate + parses VIOLATION lines from response.
// Authorized via canonicalEmmaBlock §72-amendment (rule-enforcement-
// as-emma-role).
func (re *RuleEnforcer) detectInterpretiveViolations(msgs []protocol.Message) {
	if re.client == nil {
		return
	}

	// Serialize recent msgs for prompt (compact: msg-id | from | type | content[:300]).
	var sb strings.Builder
	for _, m := range msgs {
		content := m.Content
		if len(content) > 300 {
			content = content[:300] + "...[truncated]"
		}
		fmt.Fprintf(&sb, "msg %d | from=%s | type=%s | content=%s\n",
			m.ID, m.FromAgent, m.Type, content)
	}

	// Phase-S-followup-2 F2-3: append user-custom rules from
	// ~/.bot-hq/emma/custom-rules.md if any. Decoupled ack-immediate
	// (handleMentionDirective) vs apply-at-next-tick (here) per
	// scope-lock-doc M6 design.
	prompt := fmt.Sprintf(ruleEnforcerLLMPrompt, sb.String()) + CustomRulesPromptSection()

	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()

	resp, err := re.client.Generate(ctx, prompt)
	if err != nil {
		log.Printf("rule_enforcer: LLM-judgment Generate failed: %v", err)
		return
	}

	re.parseAndEmitLLMViolations(resp)
}

// parseAndEmitLLMViolations parses the LLM response for VIOLATION
// lines and emits each. Returns count for test-assertion.
func (re *RuleEnforcer) parseAndEmitLLMViolations(resp string) int {
	if strings.Contains(resp, "NO VIOLATIONS") {
		return 0
	}
	count := 0
	for _, line := range strings.Split(resp, "\n") {
		line = strings.TrimSpace(line)
		if !strings.HasPrefix(line, "VIOLATION:") {
			continue
		}
		// Format: VIOLATION: <rule-id> | msg: <id> | <cite>
		parts := strings.SplitN(line, "|", 3)
		if len(parts) < 3 {
			continue
		}
		rule := strings.TrimSpace(strings.TrimPrefix(parts[0], "VIOLATION:"))
		msgPart := strings.TrimSpace(parts[1])
		cite := strings.TrimSpace(parts[2])

		var msgID int64
		fmt.Sscanf(msgPart, "msg: %d", &msgID)
		if msgID == 0 {
			fmt.Sscanf(msgPart, "msg:%d", &msgID)
		}

		re.emitViolation(rule, msgID, cite)
		count++
	}
	return count
}

// DetectDotParking flags heartbeat-loop antipattern: 3+ bare-`.`
// emits from a single agent within dotParkingWindow.
func (re *RuleEnforcer) DetectDotParking(msgs []protocol.Message) {
	re.detectorMu.Lock()
	defer re.detectorMu.Unlock()

	now := re.nowFunc()
	cutoff := now.Add(-dotParkingWindow)

	for _, m := range msgs {
		if m.FromAgent == ruleEnforcerAgentID {
			continue
		}
		if !dotParkingPattern.MatchString(m.Content) {
			continue
		}
		hist := re.dotParkingHistory[m.FromAgent]
		// prune old entries
		pruned := hist[:0]
		for _, t := range hist {
			if t.After(cutoff) {
				pruned = append(pruned, t)
			}
		}
		pruned = append(pruned, m.Created)
		re.dotParkingHistory[m.FromAgent] = pruned

		if len(pruned) >= dotParkingThreshold {
			cite := fmt.Sprintf("agent %s emitted %d bare-`.` within %v window — heartbeat-loop antipattern",
				m.FromAgent, len(pruned), dotParkingWindow)
			re.emitViolation("R36-DOT-PARKING", m.ID, cite)
			// reset history post-emit to prevent rapid re-fire
			re.dotParkingHistory[m.FromAgent] = nil
		}
	}
}

// DetectShapeDisclosureSkipped flags R37 estimate-shape-disclosure
// missing on emit containing an estimate phrase without a shape tag.
func (re *RuleEnforcer) DetectShapeDisclosureSkipped(msgs []protocol.Message) {
	re.detectorMu.Lock()
	defer re.detectorMu.Unlock()

	now := re.nowFunc()
	for _, m := range msgs {
		if m.FromAgent == ruleEnforcerAgentID {
			continue
		}
		if last, ok := re.shapeLastFired[m.ID]; ok && now.Sub(last) < shapeDisclosureCooldown {
			continue
		}
		if !shapeDisclosureEstimatePattern.MatchString(m.Content) {
			continue
		}
		if shapeDisclosureTagPattern.MatchString(m.Content) {
			continue
		}
		cite := fmt.Sprintf("agent %s emitted estimate without R37 shape-category tag", m.FromAgent)
		re.emitViolation("R37-SHAPE-DISCLOSURE-SKIPPED", m.ID, cite)
		re.shapeLastFired[m.ID] = now
	}
}

// DetectSnapGatingViolation flags SNAP-block emission in routine
// peer-coord context (no phase-transition keyword present).
func (re *RuleEnforcer) DetectSnapGatingViolation(msgs []protocol.Message) {
	re.detectorMu.Lock()
	defer re.detectorMu.Unlock()

	now := re.nowFunc()
	for _, m := range msgs {
		if m.FromAgent == ruleEnforcerAgentID {
			continue
		}
		if last, ok := re.snapLastFired[m.ID]; ok && now.Sub(last) < snapGatingCooldown {
			continue
		}
		if !snapBlockPattern.MatchString(m.Content) {
			continue
		}
		// Phase-transition events legitimately warrant SNAP — skip.
		if phaseTransitionPattern.MatchString(m.Content) {
			continue
		}
		cite := fmt.Sprintf("agent %s emitted SNAP-block in routine peer-coord (no phase-transition keyword)", m.FromAgent)
		re.emitViolation("R5-SNAP-GATING", m.ID, cite)
		re.snapLastFired[m.ID] = now
	}
}

// emitViolation writes a violation MsgUpdate to the hub. Per phase-
// s.md S-1b §170: hub_send broadcast only — NO system-reminder pane-
// injection. Violation emit-format follows compact-pipe convention:
// emma|violation:<rule>|msg:<id>|<cite>.
//
// MsgFlag elevation deferred — pattern-match violations default to
// MsgUpdate (informational); future scope can promote per-rule.
func (re *RuleEnforcer) emitViolation(rule string, msgID int64, cite string) {
	content := fmt.Sprintf("emma|violation:%s|msg:%d|%s", rule, msgID, cite)
	msg := protocol.Message{
		FromAgent: ruleEnforcerAgentID,
		Type:      protocol.MsgUpdate,
		Content:   content,
	}
	if _, err := re.db.InsertMessage(msg); err != nil {
		log.Printf("rule_enforcer: emitViolation InsertMessage failed: %v", err)
	}
}

// ResetStateForTest clears detector cooldown state for test isolation.
func (re *RuleEnforcer) ResetStateForTest() {
	re.detectorMu.Lock()
	defer re.detectorMu.Unlock()
	re.dotParkingHistory = make(map[string][]time.Time)
	re.shapeLastFired = make(map[int64]time.Time)
	re.snapLastFired = make(map[int64]time.Time)
}
