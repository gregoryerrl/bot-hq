package projects

import (
	"bytes"
	"fmt"

	"gopkg.in/yaml.v3"
)

// rulesCanonical is the on-disk canonical form per design-spike
// 2026-05-07-phase-n-v3.x-1.5-agent-consumption-design-spike.md §2.1:
// nested categories under gates/branch/commit; identity scalars
// (project_name + remote_url) remain flat top-level. Used exclusively
// for write-side normalization (Normalize); read-side decoding stays on
// Rules + UnmarshalYAML which accepts both forms.
//
// Pointer-typed nested categories with omitempty so empty / zero-value
// categories don't produce noise in the output.
type rulesCanonical struct {
	RemoteURL   string `yaml:"remote_url,omitempty"`
	ProjectName string `yaml:"project_name,omitempty"`

	Gates  *nestedGates  `yaml:"gates,omitempty"`
	Branch *nestedBranch `yaml:"branch,omitempty"`
	Commit *nestedCommit `yaml:"commit,omitempty"`
}

// Normalize takes raw per-project YAML bytes (potentially flat-form,
// nested-form, or mixed dual-form) and re-emits canonical nested form
// per Phase N v3.x-2 schema-canonical-form winner. Lossy on YAML
// comments + key ordering — this is intentional: the canonical-form
// closure is enforced on write (webui PUT path), so comments-as-
// authoring-tool aren't load-bearing post-normalization.
//
// Phase O drain #6: Flag A + B-prime read-side resolution via dual-form
// unmarshaler (Rules.UnmarshalYAML) is complete; this function closes
// the write-side enforcement loop so files persist in canonical form
// after any save-via-webui.
//
// Round-trip property: Normalize(Normalize(x)) = Normalize(x) for all
// valid inputs (idempotent on canonical form).
//
// Returns an error if the input is not parseable as a Rules YAML
// document. Caller (write-handler) should surface the error and reject
// the write to avoid persisting malformed content.
func Normalize(rawYAML []byte) ([]byte, error) {
	if len(bytes.TrimSpace(rawYAML)) == 0 {
		// Empty / whitespace-only → empty output is canonical-equivalent.
		return rawYAML, nil
	}

	var r Rules
	if err := yaml.Unmarshal(rawYAML, &r); err != nil {
		return nil, fmt.Errorf("normalize: parse input: %w", err)
	}

	canon := toCanonical(r)
	out, err := yaml.Marshal(canon)
	if err != nil {
		return nil, fmt.Errorf("normalize: marshal canonical: %w", err)
	}
	return out, nil
}

// toCanonical projects a Rules value into the on-disk canonical form.
// Empty / zero categories are omitted to keep the output tight; a
// category appears only if at least one of its sub-fields has a non-
// zero value.
func toCanonical(r Rules) rulesCanonical {
	c := rulesCanonical{
		RemoteURL:   r.RemoteURL,
		ProjectName: r.ProjectName,
	}

	if r.BranchPattern != "" || len(r.BranchExamples) > 0 || r.BranchPatternHelp != "" {
		c.Branch = &nestedBranch{
			Pattern:     r.BranchPattern,
			Examples:    r.BranchExamples,
			PatternHelp: r.BranchPatternHelp,
		}
	}

	if r.CommitStyle != "" || r.RequireIssueLink {
		c.Commit = &nestedCommit{
			Style:            r.CommitStyle,
			RequireIssueLink: r.RequireIssueLink,
		}
	}

	gates := nestedGates{}
	hasGates := false
	if r.PushRequiresApproval {
		gates.Push = &struct {
			RequiresApproval bool `yaml:"requiresApproval"`
		}{RequiresApproval: true}
		hasGates = true
	}
	if r.ForcePushBlocked || r.ForcePushTokenFormat != "" {
		gates.ForcePush = &struct {
			Blocked     bool   `yaml:"blocked"`
			TokenFormat string `yaml:"tokenFormat"`
		}{Blocked: r.ForcePushBlocked, TokenFormat: r.ForcePushTokenFormat}
		hasGates = true
	}
	if len(r.CoderToolsBlocked) > 0 || len(r.CoderToolsPerActionApproval) > 0 {
		gates.Coder = &struct {
			ToolsBlocked      []string `yaml:"toolsBlocked"`
			PerActionApproval []string `yaml:"perActionApproval"`
		}{
			ToolsBlocked:      r.CoderToolsBlocked,
			PerActionApproval: r.CoderToolsPerActionApproval,
		}
		hasGates = true
	}
	if hasGates {
		c.Gates = &gates
	}

	return c
}
