package protocol

import (
	"strings"
	"testing"
)

// Bug #1 ratchet: pin the four canonical literals of the DISC v2
// audience-driven routing rule. Any wording change that drops one
// of these substrings breaks the rule's substance and must fail CI
// before landing. The intentional brittleness is the test's job.
//
// If any of these literals needs to change, the change must be
// deliberate (update both the const AND the test in the same commit)
// — drift between source-of-truth and the rule's substance is
// exactly what the ratchet guards against.
func TestDiscV2OutboundRule_RatchetLiterals(t *testing.T) {
	must := []string{
		"Routing is determined by intended audience",
		"If only peer(s)",
		"Peer reads broadcasts too — never double-send",
		"If a message is both peer-coordination and user-actionable, broadcast",
	}
	for _, lit := range must {
		t.Run(lit, func(t *testing.T) {
			if !strings.Contains(DiscV2OutboundRule, lit) {
				t.Errorf("DISC v2 ratchet broken: missing literal %q (bug #1 lock)", lit)
			}
		})
	}
}

// The const must always start with the OUTBOUND header so it slots
// cleanly into agent prompts that expect the rule under that anchor.
func TestDiscV2OutboundRule_HeaderAnchor(t *testing.T) {
	if !strings.HasPrefix(DiscV2OutboundRule, "- OUTBOUND:") {
		t.Errorf("rule must start with `- OUTBOUND:` (prompt anchor); first 40 chars: %q", DiscV2OutboundRule[:40])
	}
}
