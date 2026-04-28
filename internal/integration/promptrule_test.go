// Package integration_test contains cross-package integration tests
// that ratchet behavior contracts spanning multiple bot-hq packages.
//
// promptrule_test.go (Phase J T1.4 / B5) — codifies the four manual
// prompt-rule-recognition probes performed during Phase J pass-3
// rebuild verification (msgs 4986/4988/4990/4991). These probes
// validate that agent prompts encode the load-bearing recognition
// cues for HALT class, RESUME-FROM-HALT substring, idle-on-no-SNAP
// behavior, and sentinel-FP-on-prose discrimination.
//
// Together with TestRuleNamespaceRatchet (registry-side) +
// TestPlanCapPayloadMirrorSymmetry (runtime-emit-side), this file
// closes the prompt-recognition-contract loop: const-text + runtime-
// emit + agent-prompt all share the load-bearing substrings.
package integration_test

import (
	"strings"
	"testing"

	"github.com/gregoryerrl/bot-hq/internal/brian"
	"github.com/gregoryerrl/bot-hq/internal/protocol"
	"github.com/gregoryerrl/bot-hq/internal/rain"
)

// agentPrompts returns Brian's and Rain's initial prompts for substring
// assertions. Helper to keep probe-cases concise.
func agentPrompts() (string, string) {
	b := &brian.Brian{}
	r := &rain.Rain{}
	return b.InitialPromptForTest(), r.InitialPromptForTest()
}

// TestPromptRuleRecognition_HaltClass — Phase J T1.4 probe 1.
// Locks: both agents' prompts contain BOTH context-cap and plan-cap
// halt-class trigger substrings + the H-31 HALT-ALL-WORK rule shape
// (idle in pane, do NOT close session). Cross-package: prompts
// (brian/rain) import protocol.PhaseJv1HaltResumeProtocol per T1.2.
func TestPromptRuleRecognition_HaltClass(t *testing.T) {
	bp, rp := agentPrompts()
	probes := []string{
		`agent <id> at <N>%, halt`,           // context-cap trigger
		`plan usage at <N>%, halt`,           // plan-cap trigger
		`HALT-ALL-WORK`,                      // rule name
		`idle in pane`,                       // post-Fix-3 wording
		`do NOT close the claude session`,    // load-bearing for (D) hybrid
		`stay alive to receive RESUME`,       // bootstrap contract
	}
	for _, sub := range probes {
		t.Run("brian/"+sub, func(t *testing.T) {
			if !strings.Contains(bp, sub) {
				t.Errorf("Brian prompt missing HALT-class probe substring %q", sub)
			}
		})
		t.Run("rain/"+sub, func(t *testing.T) {
			if !strings.Contains(rp, sub) {
				t.Errorf("Rain prompt missing HALT-class probe substring %q", sub)
			}
		})
	}
}

// TestPromptRuleRecognition_ResumeSubstring — Phase J T1.4 probe 2.
// Locks: both agents' prompts contain the "plan usage reset" trigger
// substring + the 3-step RESUME-FROM-HALT rule (SNAP-check, R16
// bootstrap, idle-no-engage).
func TestPromptRuleRecognition_ResumeSubstring(t *testing.T) {
	bp, rp := agentPrompts()
	probes := []string{
		`plan usage reset`,                              // trigger substring
		`RESUME-FROM-HALT`,                              // rule name
		`last-session SNAP`,                             // step 1
		`re-bootstrap via R16`,                          // step 2
		`if no SNAP exists, remain idle`,                // step 3
		`do NOT auto-engage on empty state`,             // discipline anchor
	}
	for _, sub := range probes {
		t.Run("brian/"+sub, func(t *testing.T) {
			if !strings.Contains(bp, sub) {
				t.Errorf("Brian prompt missing RESUME-substring probe %q", sub)
			}
		})
		t.Run("rain/"+sub, func(t *testing.T) {
			if !strings.Contains(rp, sub) {
				t.Errorf("Rain prompt missing RESUME-substring probe %q", sub)
			}
		})
	}
}

// TestPromptRuleRecognition_IdleOnNoSnap — Phase J T1.4 probe 3.
// Locks: idle-on-no-SNAP discipline is present AND the recognition cue
// for "would burn tokens with no work-thread" (Phase I W2 user-msg-4929
// SNAP-gate refinement rationale) survives in both prompts.
func TestPromptRuleRecognition_IdleOnNoSnap(t *testing.T) {
	bp, rp := agentPrompts()
	probes := []string{
		`if no SNAP exists, remain idle`,
		`do NOT auto-engage on empty state`,
		`would burn tokens with no work-thread`,
	}
	for _, sub := range probes {
		t.Run("brian/"+sub, func(t *testing.T) {
			if !strings.Contains(bp, sub) {
				t.Errorf("Brian prompt missing idle-on-no-SNAP probe %q", sub)
			}
		})
		t.Run("rain/"+sub, func(t *testing.T) {
			if !strings.Contains(rp, sub) {
				t.Errorf("Rain prompt missing idle-on-no-SNAP probe %q", sub)
			}
		})
	}
}

// TestPromptRuleRecognition_SentinelFpOnProse — Phase J T1.4 probe 4.
// Cross-package: ratchets that the protocol-package PayloadMirror
// substring set for H-31-HALT (the trigger substrings the agent
// matches on incoming flag messages) is sufficient to identify
// HALT-class flags amid prose noise. Couples T1.3 sentinel
// content-shape to the recognition contract.
func TestPromptRuleRecognition_SentinelFpOnProse(t *testing.T) {
	subs := protocol.PayloadMirrorSubstrings("H-31-HALT")
	if len(subs) == 0 {
		t.Fatal("H-31-HALT PayloadMirrorSubstrings empty — schema gap")
	}
	// Real-emit class — must contain ALL substrings.
	realEmit := "[CRITICAL] plan usage at 95%, halt + idle in pane"
	for _, s := range subs {
		if !strings.Contains(realEmit, s) {
			t.Errorf("real-emit text %q missing required substring %q", realEmit, s)
		}
	}
	// Prose class — must NOT contain ALL substrings (some may overlap
	// individually, but real-emit shape requires substring co-occurrence).
	proses := []string{
		"in Go runtime, what triggers a panic vs a fatal error", // no "plan usage at" / no "halt" co-occur
		"discussing the rate-limit response shape",              // no "plan usage at" / no "halt"
	}
	for _, p := range proses {
		all := true
		for _, s := range subs {
			if !strings.Contains(p, s) {
				all = false
				break
			}
		}
		if all {
			t.Errorf("prose %q matched ALL H-31-HALT substrings — would FP-elevate", p)
		}
	}
}

// TestPromptRuleRecognition_R19CycleCloseUserBlocking — Phase J T1.4
// extension probe (R19 was added in T1.1 of this same phase). Locks
// the discriminator ("would proceeding force revert") and the
// AFK-pass exhibit-of-explicit-delegation are both preserved in
// prompts. Anchors the load-bearing rule that drove pass-3 scope
// correction (msgs 5042-5049).
func TestPromptRuleRecognition_R19CycleCloseUserBlocking(t *testing.T) {
	bp, rp := agentPrompts()
	probes := []string{
		`CYCLE-CLOSE-USER-BLOCKING`,
		`would proceeding without user input force revert`,
		`unless user explicitly says otherwise`,
	}
	for _, sub := range probes {
		t.Run("brian/"+sub, func(t *testing.T) {
			if !strings.Contains(bp, sub) {
				t.Errorf("Brian prompt missing R19 probe %q", sub)
			}
		})
		t.Run("rain/"+sub, func(t *testing.T) {
			if !strings.Contains(rp, sub) {
				t.Errorf("Rain prompt missing R19 probe %q", sub)
			}
		})
	}
}
