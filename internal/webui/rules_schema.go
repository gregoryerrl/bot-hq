package webui

import (
	"fmt"

	"gopkg.in/yaml.v3"
)

// RulesGeneral is the schema for ~/.bot-hq/rules/general.yaml. Per Q-rules-6
// LOCKED: known keys validated against this Go struct; unknown keys
// allowed (warned not blocked) for forward-compat.
//
// Future extensions: add new fields here + register in expectedKeys() for
// the warning-on-unknown semantic.
type RulesGeneral struct {
	Tone       *RulesTone       `yaml:"tone,omitempty"`
	Greenlight *RulesGreenlight `yaml:"greenlight,omitempty"`
	Ratchets   *RulesRatchets   `yaml:"ratchets,omitempty"`
	HubDiscipline *RulesHubDiscipline `yaml:"hubDiscipline,omitempty"`
	Gates      *RulesGates      `yaml:"gates,omitempty"`
}

type RulesTone struct {
	Reply          string `yaml:"reply,omitempty"`
	EOD            string `yaml:"eod,omitempty"`
	Implementation string `yaml:"implementation,omitempty"`
}

type RulesGreenlight struct {
	Push      string `yaml:"push,omitempty"`
	ForcePush string `yaml:"forcePush,omitempty"`
	Merge     string `yaml:"merge,omitempty"`
}

type RulesRatchets struct {
	LocusFile string `yaml:"locusFile,omitempty"`
	Cadence   string `yaml:"cadence,omitempty"`
}

type RulesHubDiscipline struct {
	HandshakeTerminator       string `yaml:"handshakeTerminator,omitempty"`
	CrossInFlight             string `yaml:"crossInFlight,omitempty"`
	CompactFormat             string `yaml:"compactFormat,omitempty"`
	AudienceClassDiscriminator string `yaml:"audienceClassDiscriminator,omitempty"`
}

// RulesGates is the schema for the gates: top-level section of
// general.yaml. Each field holds the full markdown body of the
// corresponding ~/.bot-hq/gates/<file>.md gate-checklist as a YAML
// block scalar. R33 PRE-EXECUTE-GATE-FILE-READ toolgate enforcement
// continues to consume the on-disk files at ~/.bot-hq/gates/ — this
// schema-field is the parallel webui-readable surface (Phase O drain
// item per phase-n.md:808). Future Phase O sub-item migrates the R33
// source-of-truth to read from this field; until then disk-files
// remain authoritative for SHA-cite computation (R33 SHA-cite
// discipline preserved as required).
type RulesGates struct {
	PreCommitChecklist     string `yaml:"preCommitChecklist,omitempty"`
	PrePushChecklist       string `yaml:"prePushChecklist,omitempty"`
	PreMergeChecklist      string `yaml:"preMergeChecklist,omitempty"`
	PrePhaseCloseChecklist string `yaml:"prePhaseCloseChecklist,omitempty"`
}

// RulesProject is the schema for ~/.bot-hq/rules/projects/<project>.yaml.
// Per-project keys override general on conflict (deep-merge resolution).
// Schema is intentionally a superset of RulesGeneral — projects can shadow
// any general key, plus add project-only keys (push, voice, disguise, etc.).
type RulesProject struct {
	RulesGeneral  `yaml:",inline"`
	Push          string `yaml:"push,omitempty"`
	Voice         string `yaml:"voice,omitempty"`
	Disguise      string `yaml:"disguise,omitempty"`
	TestIsolation string `yaml:"testIsolation,omitempty"`
}

// RulesAgent is the schema for ~/.bot-hq/rules/agents/<agent>.yaml.
type RulesAgent struct {
	Role string             `yaml:"role,omitempty"`
	Exec *RulesAgentExec    `yaml:"exec,omitempty"`
}

type RulesAgentExec struct {
	PushClass    string `yaml:"pushClass,omitempty"`
	BashClass    string `yaml:"bashClass,omitempty"`
	CodeEdits    string `yaml:"codeEdits,omitempty"`
	GitOps       string `yaml:"gitOps,omitempty"`
	FileWrites   string `yaml:"fileWrites,omitempty"`
	Scope        string `yaml:"scope,omitempty"`
	ProposeDiff  string `yaml:"proposeDiff,omitempty"`
	HubWrites    string `yaml:"hubWrites,omitempty"`
}

// ValidationResult bundles parse + schema-validation results. Errors abort
// the write; Warnings surface but allow the write (forward-compat with
// new keys per Q-rules-6 LOCKED).
type ValidationResult struct {
	Errors   []string `json:"errors,omitempty"`
	Warnings []string `json:"warnings,omitempty"`
}

// HasErrors returns true when at least one parse/schema error blocked
// the write. Warnings alone do not block.
func (v ValidationResult) HasErrors() bool { return len(v.Errors) > 0 }

// validateRulesYAML parses + schema-validates a YAML payload for the
// supplied layer (general | project | agent). Unknown keys produce
// warnings, not errors. Schema-violations on known keys produce errors.
func validateRulesYAML(layer string, data []byte) ValidationResult {
	res := ValidationResult{}
	// Step 1: parse to map for unknown-key detection.
	var raw map[string]any
	if err := yaml.Unmarshal(data, &raw); err != nil {
		res.Errors = append(res.Errors, fmt.Sprintf("yaml parse: %v", err))
		return res
	}
	// Step 2: parse to typed schema for type-validation.
	switch layer {
	case "general":
		var g RulesGeneral
		if err := yaml.Unmarshal(data, &g); err != nil {
			res.Errors = append(res.Errors, fmt.Sprintf("schema (general): %v", err))
			return res
		}
		for k := range raw {
			if !generalKnown(k) {
				res.Warnings = append(res.Warnings, fmt.Sprintf("unknown key %q in general layer (allowed for forward-compat)", k))
			}
		}
	case "project":
		var p RulesProject
		if err := yaml.Unmarshal(data, &p); err != nil {
			res.Errors = append(res.Errors, fmt.Sprintf("schema (project): %v", err))
			return res
		}
		for k := range raw {
			if !projectKnown(k) {
				res.Warnings = append(res.Warnings, fmt.Sprintf("unknown key %q in project layer (allowed for forward-compat)", k))
			}
		}
	case "agent":
		var a RulesAgent
		if err := yaml.Unmarshal(data, &a); err != nil {
			res.Errors = append(res.Errors, fmt.Sprintf("schema (agent): %v", err))
			return res
		}
		for k := range raw {
			if !agentKnown(k) {
				res.Warnings = append(res.Warnings, fmt.Sprintf("unknown key %q in agent layer (allowed for forward-compat)", k))
			}
		}
	default:
		res.Errors = append(res.Errors, fmt.Sprintf("unknown rules layer %q (must be general | project | agent)", layer))
	}
	return res
}

func generalKnown(k string) bool {
	switch k {
	case "tone", "greenlight", "ratchets", "hubDiscipline", "gates":
		return true
	}
	return false
}

func projectKnown(k string) bool {
	if generalKnown(k) {
		return true
	}
	switch k {
	case "push", "voice", "disguise", "testIsolation":
		return true
	}
	return false
}

func agentKnown(k string) bool {
	switch k {
	case "role", "exec":
		return true
	}
	return false
}
