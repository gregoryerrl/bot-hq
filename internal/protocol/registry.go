package protocol

// Rule describes a single prompt-rule's complete metadata for the
// rule-namespace-ratchet test (Phase J T1.2 B3d). Each rule is enumerated
// in the Rules slice; TestRuleNamespaceRatchet iterates the slice and
// asserts the const resolves, test-locks exist, history-pointer is
// non-empty, payload-mirror substring-set matches (when present), and
// agent-applicability matches actual embed sites.
//
// Schema-design source: docs/plans/2026-04-29-rule-loci-audit.md (Rain B3a
// audit, Phase J pass-3, msg 5061). Decisions per Brian concur on Rain's
// 6 schema-design open Qs (msg 5073):
//   Q1 bundled-vs-split: ONE const PhaseJv1HaltResumeProtocol
//   Q2 sub-rule enum: registry-slice (this file)
//   Q3 agent_applicability enforcement: yes
//   Q4 R16 payload-mirror test: Option B shared-substring-set
//   Q5 history_pointer field: flexible string
//   Q6 F2 fold into B3b: yes (planCapReasonFmt cleanup co-shipped)
type Rule struct {
	// ID is the canonical identifier — e.g., "OUTBOUND", "R1", "H-13",
	// "H-31-HALT", "RESUME-FROM-HALT". Stable across edits.
	ID string

	// Name is the human-readable rule name — e.g., "HANDSHAKE-TERMINATOR",
	// "AGENT-AUTHORITY-MATRIX". May change with intentional renames; ratchet
	// tests must update with the rename to force review.
	Name string

	// ConstName is the Go-symbol path of the source const — e.g.,
	// "protocol.PhaseIv1ProtocolHardening". For sub-rules within a bundled
	// const, ConstName points at the parent and SubID identifies the sub.
	ConstName string

	// SubID is the optional sub-rule identifier within a bundled const —
	// e.g., "R1" within PhaseIv1ProtocolHardening, "HALT-ALL-WORK" within
	// PhaseJv1HaltResumeProtocol. Empty string when the rule has its own
	// dedicated const.
	SubID string

	// EmbeddedIn lists the file:line locations where agent-prompts embed
	// this rule. Used by the ratchet test to assert agent_applicability
	// matches actual embed sites (Q3 enforcement).
	EmbeddedIn []string

	// TestLockTestNames lists the Go test functions that pin this rule
	// (presence + substance). Empty slice means no test-lock exists yet
	// (acceptable for newly-added rules pending lock).
	TestLockTestNames []string

	// HistoryPointer is the location of rationale + msg-ID history. May
	// be a Go-comment loc (e.g., "internal/protocol/disc.go:25-62") or a
	// docs/arcs/-path (T2.1 may migrate Go-comments → docs/arcs/ files).
	// Flexible per Q5; ratchet asserts non-empty only.
	HistoryPointer string

	// PayloadMirror is the optional file:line:identifier of a runtime-emit
	// equivalent — e.g., "internal/gemma/plan_usage.go:59:planCapResumeFmt"
	// for RESUME-FROM-HALT. When non-empty, the ratchet test asserts the
	// const text and the runtime-emit string share a substring-set
	// (Option B per Q4). Empty means no runtime-emit equivalent.
	PayloadMirror string

	// AgentApplicability lists which agents embed this rule. Valid values:
	// "brian", "rain". Mismatch with EmbeddedIn → ratchet test fails (Q3).
	AgentApplicability []string
}

// Rules is the canonical enumeration of all prompt-rules across the bot-hq
// trio's prompt surface. Source-of-truth for TestRuleNamespaceRatchet
// (registry_test.go).
//
// Maintenance: when adding a new rule (const-shared OR inline), ALSO add
// an entry here. The ratchet test asserts every embed site has a
// corresponding registry entry and vice versa.
//
// Substrate: Rain B3a audit (docs/plans/2026-04-29-rule-loci-audit.md)
// enumerated 19 rules across the prompt surface as of Phase J pass-3
// kickoff. Phase J pass-3 adds R17/R18/R19 in T1.1 (PhaseIv1 const
// additions); registry entries for those land with that work.
var Rules = []Rule{
	{
		ID:                 "OUTBOUND",
		Name:               "OUTBOUND-EVERY-REPLY-IS-HUB-SEND",
		ConstName:          "protocol.DiscV2OutboundRule",
		EmbeddedIn:         []string{"internal/brian/brian.go:260", "internal/rain/rain.go:246"},
		TestLockTestNames:  []string{"TestInitialPromptEmbedsDiscV2OutboundRule", "TestDiscV2OutboundRule_RatchetLiterals", "TestDiscV2OutboundRule_HeaderAnchor"},
		HistoryPointer:     "internal/protocol/disc.go:3-20",
		AgentApplicability: []string{"brian", "rain"},
	},
	// PhaseIv1ProtocolHardening sub-rules R1-R16 — bundled const.
	{ID: "R1", Name: "HANDSHAKE-TERMINATOR", ConstName: "protocol.PhaseIv1ProtocolHardening", SubID: "R1", EmbeddedIn: []string{"internal/brian/brian.go:261", "internal/rain/rain.go:247"}, TestLockTestNames: []string{"TestPhaseIv1ContentShape"}, HistoryPointer: "internal/protocol/disc.go:25-62", AgentApplicability: []string{"brian", "rain"}},
	{ID: "R2", Name: "CROSS-TIMING-DEDUP", ConstName: "protocol.PhaseIv1ProtocolHardening", SubID: "R2", EmbeddedIn: []string{"internal/brian/brian.go:261", "internal/rain/rain.go:247"}, TestLockTestNames: []string{"TestPhaseIv1ContentShape"}, HistoryPointer: "internal/protocol/disc.go:25-62", AgentApplicability: []string{"brian", "rain"}},
	{ID: "R3", Name: "QUOTE-TRIM", ConstName: "protocol.PhaseIv1ProtocolHardening", SubID: "R3", EmbeddedIn: []string{"internal/brian/brian.go:261", "internal/rain/rain.go:247"}, TestLockTestNames: []string{"TestPhaseIv1ContentShape"}, HistoryPointer: "internal/protocol/disc.go:25-62", AgentApplicability: []string{"brian", "rain"}},
	{ID: "R4", Name: "SNAP-GATING", ConstName: "protocol.PhaseIv1ProtocolHardening", SubID: "R4", EmbeddedIn: []string{"internal/brian/brian.go:261", "internal/rain/rain.go:247"}, TestLockTestNames: []string{"TestPhaseIv1ContentShape"}, HistoryPointer: "internal/protocol/disc.go:25-62", AgentApplicability: []string{"brian", "rain"}},
	{ID: "R5", Name: "BRAIN-CYCLE-RESPONSE-SHAPE", ConstName: "protocol.PhaseIv1ProtocolHardening", SubID: "R5", EmbeddedIn: []string{"internal/brian/brian.go:261", "internal/rain/rain.go:247"}, TestLockTestNames: []string{"TestPhaseIv1ContentShape"}, HistoryPointer: "internal/protocol/disc.go:25-62", AgentApplicability: []string{"brian", "rain"}},
	{ID: "R6", Name: "TOOL-RESULT-DISCIPLINE", ConstName: "protocol.PhaseIv1ProtocolHardening", SubID: "R6", EmbeddedIn: []string{"internal/brian/brian.go:261", "internal/rain/rain.go:247"}, TestLockTestNames: []string{"TestPhaseIv1ContentShape"}, HistoryPointer: "internal/protocol/disc.go:25-62", AgentApplicability: []string{"brian", "rain"}},
	{ID: "R7", Name: "SUBAGENT-DISPATCH", ConstName: "protocol.PhaseIv1ProtocolHardening", SubID: "R7", EmbeddedIn: []string{"internal/brian/brian.go:261", "internal/rain/rain.go:247"}, TestLockTestNames: []string{"TestPhaseIv1ContentShape"}, HistoryPointer: "internal/protocol/disc.go:25-62", AgentApplicability: []string{"brian", "rain"}},
	{ID: "R8", Name: "COMPACT-COMMIT-FORMAT", ConstName: "protocol.PhaseIv1ProtocolHardening", SubID: "R8", EmbeddedIn: []string{"internal/brian/brian.go:261", "internal/rain/rain.go:247"}, TestLockTestNames: []string{"TestPhaseIv1ContentShape"}, HistoryPointer: "internal/protocol/disc.go:25-62", AgentApplicability: []string{"brian", "rain"}},
	{ID: "R9", Name: "AUDIENCE-CLASS-DISCRIMINATOR", ConstName: "protocol.PhaseIv1ProtocolHardening", SubID: "R9", EmbeddedIn: []string{"internal/brian/brian.go:261", "internal/rain/rain.go:247"}, TestLockTestNames: []string{"TestPhaseIv1ContentShape"}, HistoryPointer: "internal/protocol/disc.go:25-62", AgentApplicability: []string{"brian", "rain"}},
	{ID: "R10", Name: "SCOPE-LOCK-BEFORE-IMPL", ConstName: "protocol.PhaseIv1ProtocolHardening", SubID: "R10", EmbeddedIn: []string{"internal/brian/brian.go:261", "internal/rain/rain.go:247"}, TestLockTestNames: []string{"TestPhaseIv1ContentShape"}, HistoryPointer: "internal/protocol/disc.go:25-62", AgentApplicability: []string{"brian", "rain"}},
	{ID: "R11", Name: "HALT-DISCIPLINE", ConstName: "protocol.PhaseIv1ProtocolHardening", SubID: "R11", EmbeddedIn: []string{"internal/brian/brian.go:261", "internal/rain/rain.go:247"}, TestLockTestNames: []string{"TestPhaseIv1ContentShape"}, HistoryPointer: "internal/protocol/disc.go:25-62", AgentApplicability: []string{"brian", "rain"}},
	{ID: "R12", Name: "GATE-PROTOCOL", ConstName: "protocol.PhaseIv1ProtocolHardening", SubID: "R12", EmbeddedIn: []string{"internal/brian/brian.go:261", "internal/rain/rain.go:247"}, TestLockTestNames: []string{"TestPhaseIv1ContentShape"}, HistoryPointer: "internal/protocol/disc.go:25-62", AgentApplicability: []string{"brian", "rain"}},
	{ID: "R13", Name: "SCOPE-VERIFY-PRE-DRAFT", ConstName: "protocol.PhaseIv1ProtocolHardening", SubID: "R13", EmbeddedIn: []string{"internal/brian/brian.go:261", "internal/rain/rain.go:247"}, TestLockTestNames: []string{"TestPhaseIv1ContentShape"}, HistoryPointer: "internal/protocol/disc.go:25-62", AgentApplicability: []string{"brian", "rain"}},
	{ID: "R14", Name: "HALT-95%-SNAP", ConstName: "protocol.PhaseIv1ProtocolHardening", SubID: "R14", EmbeddedIn: []string{"internal/brian/brian.go:261", "internal/rain/rain.go:247"}, TestLockTestNames: []string{"TestPhaseIv1ContentShape"}, HistoryPointer: "internal/protocol/disc.go:25-62", AgentApplicability: []string{"brian", "rain"}},
	{ID: "R15", Name: "AGENT-AUTHORITY-MATRIX", ConstName: "protocol.PhaseIv1ProtocolHardening", SubID: "R15", EmbeddedIn: []string{"internal/brian/brian.go:261", "internal/rain/rain.go:247"}, TestLockTestNames: []string{"TestPhaseIv1ContentShape"}, HistoryPointer: "internal/protocol/disc.go:25-62", AgentApplicability: []string{"brian", "rain"}},
	{
		ID:                 "R16",
		Name:               "CROSS-RESTART-RESUME-OPERATIONAL",
		ConstName:          "protocol.PhaseIv1ProtocolHardening",
		SubID:              "R16",
		EmbeddedIn:         []string{"internal/brian/brian.go:261", "internal/rain/rain.go:247"},
		TestLockTestNames:  []string{"TestPhaseIv1ContentShape"},
		HistoryPointer:     "internal/protocol/disc.go:25-62",
		PayloadMirror:      "internal/gemma/plan_usage.go:59:planCapResumeFmt",
		AgentApplicability: []string{"brian", "rain"},
	},
	// Phase J T1.1 const additions: R17/R18/R19 added per pass-3 user
	// scope-correction (msgs 5042-5049) + pass-3 AFK-pass exhibit
	// (msgs 5060-5067).
	{ID: "R17", Name: "SOURCE-OF-TRUTH-HIERARCHY", ConstName: "protocol.PhaseIv1ProtocolHardening", SubID: "R17", EmbeddedIn: []string{"internal/brian/brian.go:261", "internal/rain/rain.go:247"}, TestLockTestNames: []string{"TestPhaseIv1ContentShape"}, HistoryPointer: "internal/protocol/disc.go:25-62", AgentApplicability: []string{"brian", "rain"}},
	{ID: "R18", Name: "CITE-ANCHOR-REQUIRED", ConstName: "protocol.PhaseIv1ProtocolHardening", SubID: "R18", EmbeddedIn: []string{"internal/brian/brian.go:261", "internal/rain/rain.go:247"}, TestLockTestNames: []string{"TestPhaseIv1ContentShape"}, HistoryPointer: "internal/protocol/disc.go:25-62", AgentApplicability: []string{"brian", "rain"}},
	{ID: "R19", Name: "CYCLE-CLOSE-USER-BLOCKING", ConstName: "protocol.PhaseIv1ProtocolHardening", SubID: "R19", EmbeddedIn: []string{"internal/brian/brian.go:261", "internal/rain/rain.go:247"}, TestLockTestNames: []string{"TestPhaseIv1ContentShape"}, HistoryPointer: "internal/protocol/disc.go:25-62", AgentApplicability: []string{"brian", "rain"}},
	{ID: "R20", Name: "BOOTSTRAP-ON-CONVERSATION-RESUME", ConstName: "protocol.PhaseIv1ProtocolHardening", SubID: "R20", EmbeddedIn: []string{"internal/brian/brian.go:261", "internal/rain/rain.go:247"}, TestLockTestNames: []string{"TestPhaseIv1ContentShape", "TestAgentStateRoundTrip"}, HistoryPointer: "internal/protocol/disc.go:25-62", AgentApplicability: []string{"brian", "rain"}},
	{ID: "R21", Name: "MSG-TYPE-TAXONOMY", ConstName: "protocol.PhaseIv1ProtocolHardening", SubID: "R21", EmbeddedIn: []string{"internal/brian/brian.go:261", "internal/rain/rain.go:247"}, TestLockTestNames: []string{"TestPhaseIv1ContentShape", "TestMessageTypeTaxonomy"}, HistoryPointer: "internal/protocol/disc.go:25-62", AgentApplicability: []string{"brian", "rain"}},
	{
		ID:                 "H-13",
		Name:               "FORCE-PUSH-TOKEN-PROTOCOL",
		ConstName:          "protocol.H13ForcePushProtocol",
		EmbeddedIn:         []string{"internal/brian/brian.go:287"},
		TestLockTestNames:  []string{"TestInitialPromptContainsH13ForcePushProtocol"},
		HistoryPointer:     "internal/protocol/disc.go:93-104",
		AgentApplicability: []string{"brian"},
	},
	// PhaseJv1HaltResumeProtocol sub-rules — bundled const (Q1 single-const lean).
	{
		ID:                 "H-31-HALT",
		Name:               "HALT-ALL-WORK",
		ConstName:          "protocol.PhaseJv1HaltResumeProtocol",
		SubID:              "HALT-ALL-WORK",
		EmbeddedIn:         []string{"internal/brian/brian.go:284", "internal/rain/rain.go:265"},
		TestLockTestNames:  []string{"TestBrianPromptContainsHaltAllWork", "TestRainPromptContainsHaltAllWork"},
		HistoryPointer:     "internal/protocol/disc.go:113-128",
		PayloadMirror:      "internal/gemma/plan_usage.go:48:planCapReasonFmt",
		AgentApplicability: []string{"brian", "rain"},
	},
	{
		ID:                 "RESUME-FROM-HALT",
		Name:               "RESUME-FROM-HALT",
		ConstName:          "protocol.PhaseJv1HaltResumeProtocol",
		SubID:              "RESUME-FROM-HALT",
		EmbeddedIn:         []string{"internal/brian/brian.go:284", "internal/rain/rain.go:265"},
		TestLockTestNames:  []string{"TestBrianPromptContainsResumeFromHalt", "TestRainPromptContainsResumeFromHalt"},
		HistoryPointer:     "internal/protocol/disc.go:113-128",
		PayloadMirror:      "internal/gemma/plan_usage.go:59:planCapResumeFmt",
		AgentApplicability: []string{"brian", "rain"},
	},
}

// PayloadMirrorSubstrings returns the load-bearing substring set that must
// appear in BOTH the const-text source and the runtime-emit payload string
// for a rule with PayloadMirror set. Used by TestRuleNamespaceRatchet
// (Option B shared-substring-set per Q4 lean).
//
// Substring sets per rule (locked by code, not derived) — keep tight to
// the load-bearing semantic shape. Adding tokens makes the test stricter;
// removing tokens loosens. Edits require explicit review.
func PayloadMirrorSubstrings(ruleID string) []string {
	switch ruleID {
	case "R16", "RESUME-FROM-HALT":
		// Bootstrap-order shared substrings for R16 / RESUME-FROM-HALT —
		// const text vs planCapResumeFmt (plan_usage.go:59). Locks the
		// 4-step shape (a)/(b)/(c)/(d) and the file pointers, allows
		// wording drift around them.
		return []string{
			"git status",
			"phase/<active-phase>.md",
			"ratchets/active.md",
			"hub_read",
		}
	case "H-31-HALT":
		// Trigger-substring shared between H-31 prompt rule and
		// planCapReasonFmt emitter. The agent matches "plan usage at" +
		// "halt" substring; emitter must contain both.
		return []string{
			"plan usage at",
			"halt",
		}
	}
	return nil
}
