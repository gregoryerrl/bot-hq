package webui

import (
	"strings"
	"testing"
)

// TestValidateRulesYAML_GeneralAllKnownKeys: well-formed general.yaml
// with all schema-recognized keys (incl. Phase O drain #2a hubDiscipline
// gap-1 fill-in) parses with zero errors + zero warnings.
func TestValidateRulesYAML_GeneralAllKnownKeys(t *testing.T) {
	yaml := []byte(`
tone:
  reply: "concise + cite-anchors"
  eod: "compact pipe-format"
  implementation: "TDD discipline"

greenlight:
  push: "explicit user verbatim"
  forcePush: "R29 elevated gate"
  merge: "user-only ABSOLUTE"

ratchets:
  locusFile: "~/.bot-hq/ratchets/active.md"
  cadence: "per-phase"

hubDiscipline:
  handshakeTerminator: "single '.' on no-content + no-action"
  crossInFlight: "emit '[crossed in flight - see msg N]' instead of full repost"
  compactFormat: "agent-to-agent: pipe-separated sender|event:value|key:value"
  audienceClassDiscriminator: "[HR] prefix marks must-read; default untagged compact"
`)
	res := validateRulesYAML("general", yaml)
	if res.HasErrors() {
		t.Fatalf("unexpected errors: %v", res.Errors)
	}
	if len(res.Warnings) > 0 {
		t.Errorf("unexpected warnings: %v", res.Warnings)
	}
}

// TestValidateRulesYAML_HubDisciplineGap1Keys: the two Phase O drain #2a
// keys (compactFormat + audienceClassDiscriminator) round-trip through
// the schema struct without data loss.
func TestValidateRulesYAML_HubDisciplineGap1Keys(t *testing.T) {
	yaml := []byte(`
hubDiscipline:
  compactFormat: "compact pipe-separated for peer-coord"
  audienceClassDiscriminator: "[HR] for must-read"
`)
	res := validateRulesYAML("general", yaml)
	if res.HasErrors() {
		t.Fatalf("errors on gap-1 keys: %v", res.Errors)
	}
	for _, w := range res.Warnings {
		if strings.Contains(w, "compactFormat") || strings.Contains(w, "audienceClassDiscriminator") {
			t.Errorf("gap-1 key flagged as unknown: %q", w)
		}
	}
}

// TestValidateRulesYAML_LegacyHubDisciplineKeysStillWork: pre-#2a keys
// (handshakeTerminator + crossInFlight) continue to parse cleanly. Guards
// against accidental schema regression in the gap-1 fill-in.
func TestValidateRulesYAML_LegacyHubDisciplineKeysStillWork(t *testing.T) {
	yaml := []byte(`
hubDiscipline:
  handshakeTerminator: "single dot"
  crossInFlight: "see msg N gloss"
`)
	res := validateRulesYAML("general", yaml)
	if res.HasErrors() {
		t.Fatalf("regression on legacy keys: %v", res.Errors)
	}
	if len(res.Warnings) > 0 {
		t.Errorf("unexpected warnings on legacy keys: %v", res.Warnings)
	}
}

// TestValidateRulesYAML_UnknownTopLevelStillWarns: forward-compat Q-rules-6
// behavior preserved — unknown top-level keys produce warning, not error.
func TestValidateRulesYAML_UnknownTopLevelStillWarns(t *testing.T) {
	yaml := []byte(`
tone:
  reply: ok
futureTopLevelKey: "not-yet-schemafied"
`)
	res := validateRulesYAML("general", yaml)
	if res.HasErrors() {
		t.Fatalf("unknown key should warn not error: %v", res.Errors)
	}
	if len(res.Warnings) == 0 {
		t.Errorf("expected warning for unknown key, got none")
	}
	found := false
	for _, w := range res.Warnings {
		if strings.Contains(w, "futureTopLevelKey") {
			found = true
		}
	}
	if !found {
		t.Errorf("warning didn't mention futureTopLevelKey: %v", res.Warnings)
	}
}

// TestValidateRulesYAML_ProjectInheritsHubDisciplineGap1: per-project
// rules.yaml can use the same hubDiscipline keys as general (RulesProject
// embeds RulesGeneral). Confirms gap-1 fill-in is inherited.
func TestValidateRulesYAML_ProjectInheritsHubDisciplineGap1(t *testing.T) {
	yaml := []byte(`
hubDiscipline:
  compactFormat: "project-specific override"
  audienceClassDiscriminator: "project-specific [HR] policy"
push: "explicit user verbatim 'push X'"
`)
	res := validateRulesYAML("project", yaml)
	if res.HasErrors() {
		t.Fatalf("project-layer gap-1 keys errored: %v", res.Errors)
	}
	if len(res.Warnings) > 0 {
		t.Errorf("project-layer gap-1 keys warned: %v", res.Warnings)
	}
}

// TestValidateRulesYAML_GatesSchemaField: Phase O drain #3 gates schema-
// field accepts block-scalar markdown content for all 4 known gate files.
// R33 source-of-truth still on-disk; this field is the webui-readable
// surface only. No errors, no warnings.
func TestValidateRulesYAML_GatesSchemaField(t *testing.T) {
	yaml := []byte(`
gates:
  preCommitChecklist: |
    # Pre-commit checklist
    - item 1
    - item 2
  prePushChecklist: |
    # Pre-push checklist
    - item 1
  preMergeChecklist: |
    # Pre-merge checklist (USER-ONLY ABSOLUTE)
    - item 1
  prePhaseCloseChecklist: |
    # Pre-phase-close checklist
    - item 1
`)
	res := validateRulesYAML("general", yaml)
	if res.HasErrors() {
		t.Fatalf("unexpected errors: %v", res.Errors)
	}
	for _, w := range res.Warnings {
		if strings.Contains(w, "gates") {
			t.Errorf("gates flagged as unknown: %q", w)
		}
	}
}

// TestValidateRulesYAML_GatesPartialPopulation: gates field is optional
// per-key — populating only preCommitChecklist (other 3 omitted) still
// validates clean. Models incremental migration paths.
func TestValidateRulesYAML_GatesPartialPopulation(t *testing.T) {
	yaml := []byte(`
gates:
  preCommitChecklist: "single-key population test"
`)
	res := validateRulesYAML("general", yaml)
	if res.HasErrors() {
		t.Fatalf("partial population errored: %v", res.Errors)
	}
	if len(res.Warnings) > 0 {
		t.Errorf("partial population warned: %v", res.Warnings)
	}
}

// TestValidateRulesYAML_GatesUnknownSubkeyTypeError: subkey type
// mismatch (map instead of string) errors out — schema enforces string-
// only block-scalar shape.
func TestValidateRulesYAML_GatesUnknownSubkeyTypeError(t *testing.T) {
	yaml := []byte(`
gates:
  preCommitChecklist:
    nested: "should-not-parse"
`)
	res := validateRulesYAML("general", yaml)
	if !res.HasErrors() {
		t.Errorf("expected schema error on map-instead-of-string, got none")
	}
}

// TestValidateRulesYAML_GreenlightAbsoluteCoversPush: Phase O drain
// codifies absolute-greenlight-covers-push semantics per phase-n.md:810
// + user msg 8772. Field is a bool with omitempty — accepts true/false
// + omission. Validates parse-clean across all 3 forms.
func TestValidateRulesYAML_GreenlightAbsoluteCoversPush(t *testing.T) {
	cases := []struct {
		name string
		yaml []byte
	}{
		{"true-own-repo", []byte("greenlight:\n  absoluteCoversPush: true\n")},
		{"false-client-repo", []byte("greenlight:\n  absoluteCoversPush: false\n")},
		{"omitted-defaults-strict", []byte("greenlight:\n  push: \"explicit verbatim\"\n")},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			res := validateRulesYAML("general", tc.yaml)
			if res.HasErrors() {
				t.Fatalf("unexpected errors: %v", res.Errors)
			}
			for _, w := range res.Warnings {
				if strings.Contains(w, "absoluteCoversPush") {
					t.Errorf("absoluteCoversPush flagged unknown: %q", w)
				}
			}
		})
	}
}

// TestValidateRulesYAML_GreenlightAbsoluteCoversPushTypeMismatch: bool
// field rejects string value — schema enforces type.
func TestValidateRulesYAML_GreenlightAbsoluteCoversPushTypeMismatch(t *testing.T) {
	yaml := []byte(`
greenlight:
  absoluteCoversPush: "not-a-bool"
`)
	res := validateRulesYAML("general", yaml)
	if !res.HasErrors() {
		t.Errorf("expected schema error on non-bool absoluteCoversPush, got none")
	}
}

// TestValidateRulesYAML_BadYAML: malformed input → error, blocks write.
func TestValidateRulesYAML_BadYAML(t *testing.T) {
	yaml := []byte("not valid yaml: : :")
	res := validateRulesYAML("general", yaml)
	if !res.HasErrors() {
		t.Errorf("expected errors on malformed yaml, got none")
	}
}

// TestValidateRulesYAML_UnknownLayer: caller-side guard — unsupported
// layer name → error.
func TestValidateRulesYAML_UnknownLayer(t *testing.T) {
	res := validateRulesYAML("nonsense", []byte("tone: {reply: x}"))
	if !res.HasErrors() {
		t.Errorf("expected errors for unknown layer, got none")
	}
}
