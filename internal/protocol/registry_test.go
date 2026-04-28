package protocol

import (
	"strings"
	"testing"
)

// TestRuleNamespaceRatchet enumerates the Rules registry and asserts
// each rule's invariants. Phase J T1.2 B3d schema-establishment.
//
// Invariants asserted (per Brian concur on Rain B3a audit §5 leans):
//   - Q1 single-const for paired rules (PhaseJv1HaltResumeProtocol)
//   - Q2 registry-slice enumeration (this file)
//   - Q3 agent_applicability matches embed-site path prefix
//   - Q4 R16 payload-mirror Option B (shared-substring-set) — checked here
//     against the const-text source; runtime-emit symmetry via the
//     companion test in internal/gemma/plan_usage_test.go
//   - Q5 history_pointer field flexible (string)
//   - Q6 F2 fold (planCapReasonFmt cleanup) — in disc.go const + plan_usage.go const
func TestRuleNamespaceRatchet(t *testing.T) {
	if len(Rules) == 0 {
		t.Fatal("Rules registry is empty — must enumerate prompt-rule surface")
	}

	seenIDs := map[string]bool{}
	for _, r := range Rules {
		t.Run(r.ID, func(t *testing.T) {
			// Basic invariants
			if r.ID == "" {
				t.Error("rule ID must be non-empty")
			}
			if seenIDs[r.ID] {
				t.Errorf("duplicate rule ID %q in Rules registry", r.ID)
			}
			seenIDs[r.ID] = true

			if r.Name == "" {
				t.Errorf("rule %q: Name must be non-empty", r.ID)
			}
			if r.ConstName == "" {
				t.Errorf("rule %q: ConstName must be non-empty", r.ID)
			}
			if r.HistoryPointer == "" {
				t.Errorf("rule %q: HistoryPointer must be non-empty (Q5 — flexible string, but presence required)", r.ID)
			}
			if len(r.AgentApplicability) == 0 {
				t.Errorf("rule %q: AgentApplicability must list at least one agent", r.ID)
			}
			for _, a := range r.AgentApplicability {
				if a != "brian" && a != "rain" {
					t.Errorf("rule %q: AgentApplicability has invalid agent %q (must be 'brian' or 'rain')", r.ID, a)
				}
			}
			if len(r.EmbeddedIn) == 0 {
				t.Errorf("rule %q: EmbeddedIn must list at least one file:line location", r.ID)
			}

			// Q3 agent_applicability cross-check: embed-site file path must
			// contain the agent ID (internal/brian/ for "brian", internal/rain/ for "rain").
			for _, applicableAgent := range r.AgentApplicability {
				wantPath := "internal/" + applicableAgent + "/"
				found := false
				for _, embed := range r.EmbeddedIn {
					if strings.Contains(embed, wantPath) {
						found = true
						break
					}
				}
				if !found {
					t.Errorf("rule %q: AgentApplicability lists %q but no EmbeddedIn entry contains %q", r.ID, applicableAgent, wantPath)
				}
			}
			// Reverse check: every embed-site agent path must be in AgentApplicability.
			for _, embed := range r.EmbeddedIn {
				if strings.Contains(embed, "internal/brian/") && !contains(r.AgentApplicability, "brian") {
					t.Errorf("rule %q: EmbeddedIn has %q but AgentApplicability does not list 'brian'", r.ID, embed)
				}
				if strings.Contains(embed, "internal/rain/") && !contains(r.AgentApplicability, "rain") {
					t.Errorf("rule %q: EmbeddedIn has %q but AgentApplicability does not list 'rain'", r.ID, embed)
				}
			}

			// Q4 payload-mirror substring-set (Option B). When PayloadMirror
			// is set, the const-text source must contain all substrings.
			// Runtime-emit symmetry checked in gemma-side test.
			if r.PayloadMirror != "" {
				substrings := PayloadMirrorSubstrings(r.ID)
				if len(substrings) == 0 {
					t.Errorf("rule %q: PayloadMirror set but PayloadMirrorSubstrings returned no substrings — schema gap", r.ID)
				}
				constText := constTextFor(r.ConstName)
				if constText == "" {
					t.Errorf("rule %q: cannot resolve ConstName %q for substring check", r.ID, r.ConstName)
					return
				}
				for _, sub := range substrings {
					if !strings.Contains(constText, sub) {
						t.Errorf("rule %q: const-text %q missing payload-mirror substring %q", r.ID, r.ConstName, sub)
					}
				}
			}
		})
	}
}

// constTextFor returns the body of a const referenced by string name. Limited
// to the protocol package's known consts. Used by the ratchet test for
// payload-mirror substring-set assertions.
func constTextFor(constName string) string {
	switch constName {
	case "protocol.DiscV2OutboundRule":
		return DiscV2OutboundRule
	case "protocol.PhaseIv1ProtocolHardening":
		return PhaseIv1ProtocolHardening
	case "protocol.PhaseJv1HaltResumeProtocol":
		return PhaseJv1HaltResumeProtocol
	case "protocol.H13ForcePushProtocol":
		return H13ForcePushProtocol
	}
	return ""
}

func contains(haystack []string, needle string) bool {
	for _, h := range haystack {
		if h == needle {
			return true
		}
	}
	return false
}

// TestRuleRegistryCoversAllConstRules sanity-checks that every Rule with
// SubID="" maps to a known const, and PhaseIv1 sub-rules cover the rule
// names locked in TestPhaseIv1ContentShape (cross-check with brian_test.go).
//
// Phase I has 16 sub-rules (R1-R16); Phase J pass-3 adds R17/R18/R19 in
// T1.1 (registry entries land with that work). Until then, R1-R16 only.
func TestRuleRegistryCoversAllConstRules(t *testing.T) {
	phaseIRuleNames := map[string]bool{}
	for _, r := range Rules {
		if r.ConstName == "protocol.PhaseIv1ProtocolHardening" {
			phaseIRuleNames[r.Name] = true
		}
	}

	// Locked at Phase J T1.1 close — Phase I R1-R16 + Phase J T1.1 R17-R19.
	mustHave := []string{
		"HANDSHAKE-TERMINATOR",
		"CROSS-TIMING-DEDUP",
		"QUOTE-TRIM",
		"SNAP-GATING",
		"BRAIN-CYCLE-RESPONSE-SHAPE",
		"TOOL-RESULT-DISCIPLINE",
		"SUBAGENT-DISPATCH",
		"COMPACT-COMMIT-FORMAT",
		"AUDIENCE-CLASS-DISCRIMINATOR",
		"SCOPE-LOCK-BEFORE-IMPL",
		"HALT-DISCIPLINE",
		"GATE-PROTOCOL",
		"SCOPE-VERIFY-PRE-DRAFT",
		"HALT-95%-SNAP",
		"AGENT-AUTHORITY-MATRIX",
		"CROSS-RESTART-RESUME-OPERATIONAL",
		// Phase J T1.1 additions
		"SOURCE-OF-TRUTH-HIERARCHY",
		"CITE-ANCHOR-REQUIRED",
		"CYCLE-CLOSE-USER-BLOCKING",
		// Phase J T1.5 addition
		"BOOTSTRAP-ON-CONVERSATION-RESUME",
		// Phase J T1.7 addition
		"MSG-TYPE-TAXONOMY",
		// Phase J T2.2 addition
		"PRE-COMPACT-SNAP",
	}
	for _, name := range mustHave {
		if !phaseIRuleNames[name] {
			t.Errorf("Rules registry missing PhaseIv1 sub-rule %q", name)
		}
	}
}
