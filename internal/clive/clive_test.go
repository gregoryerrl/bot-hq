package clive

// Tests for Clive's prompt-build integration (P-4a / phase-n.md:545 +
// PhaseNv3CliveExpansion at protocol/disc.go:543). Mirrors the rain-
// package ratchet pattern (TestInitialPromptContainsDISCv2): pin the
// canonical literals so the prompt cannot drift away from rule-text
// without breaking these tests.

import (
	"strings"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/protocol"
)

// TestInitialPromptContainsCliveExpansion locks the load-bearing fact
// that PhaseNv3CliveExpansion is embedded in Clive's initial prompt.
// Without this, the const stays orphaned — the v3c-deferred wiring
// would silently no-op.
func TestInitialPromptContainsCliveExpansion(t *testing.T) {
	prompt := InitialPrompt()
	// Pin a substring early in the rule that's stable + identifies
	// the const uniquely (avoids false-pass via partial-match drift).
	wants := []string{
		"CLIVE (v3c expansion):",
		"plan-cooperator + draft-author + diff-proposer",
		"canonical-store-write-API-caller",
		"POST /api/files/{path}/clive",
		"Cannot bare-filesystem-write",
	}
	for _, w := range wants {
		if !strings.Contains(prompt, w) {
			t.Errorf("initial prompt must contain CliveExpansion literal %q (orphan-const regression risk)", w)
		}
	}
}

// TestInitialPromptContainsDISCv2 mirrors rain_test pattern: ensure the
// shared DISC v2 base + outbound discipline are part of Clive's prompt
// so handshake / handlock / outbound rules apply uniformly.
func TestInitialPromptContainsDISCv2(t *testing.T) {
	prompt := InitialPrompt()
	wants := []string{
		"DISC v2 2026-04-24:",
		protocol.DiscV2OutboundRule[:60], // first 60 chars stable across versions
	}
	for _, w := range wants {
		if !strings.Contains(prompt, w) {
			t.Errorf("initial prompt must contain DISC v2 literal %q", w)
		}
	}
}

// TestInitialPromptContainsR36Mechanical pins R36 OUTBOUND-DISCIPLINE-
// MECHANICAL since outbound-miss enforcement applies to all trio
// agents uniformly (Clive emissions must carry hub_send same as
// Brian / Rain).
func TestInitialPromptContainsR36Mechanical(t *testing.T) {
	prompt := InitialPrompt()
	if !strings.Contains(prompt, "OUTBOUND-DISCIPLINE-MECHANICAL") {
		t.Errorf("initial prompt missing R36 OUTBOUND-DISCIPLINE-MECHANICAL anchor")
	}
}

// TestInitialPromptHubRegisterStartup locks the hub_register-as-first-
// action requirement: without this Clive could emit hub_send before
// register and get rejected.
func TestInitialPromptHubRegisterStartup(t *testing.T) {
	prompt := InitialPrompt()
	if !strings.Contains(prompt, `hub_register id="clive"`) {
		t.Errorf("initial prompt must instruct hub_register id=\"clive\" at startup")
	}
}

// TestInitialPromptCanonicalStoreScope locks the canonical-store path
// scope so future edits can't accidentally widen Clive's authority
// without breaking this ratchet.
func TestInitialPromptCanonicalStoreScope(t *testing.T) {
	prompt := InitialPrompt()
	wants := []string{
		"~/.bot-hq/{phase,ratchets,projects,rules}",
		"discipline-log.md",
		"NEVER touch code",
	}
	for _, w := range wants {
		if !strings.Contains(prompt, w) {
			t.Errorf("initial prompt must contain canonical-store-scope anchor %q", w)
		}
	}
}

// TestAgentConstants pins the canonical agent identity used for
// hub_register + session-open API.
func TestAgentConstants(t *testing.T) {
	if AgentID != "clive" {
		t.Errorf("AgentID = %q, want %q", AgentID, "clive")
	}
	if AgentName != "Clive" {
		t.Errorf("AgentName = %q, want %q", AgentName, "Clive")
	}
	if AgentType != protocol.AgentVoice {
		t.Errorf("AgentType = %q, want %q", AgentType, protocol.AgentVoice)
	}
}
