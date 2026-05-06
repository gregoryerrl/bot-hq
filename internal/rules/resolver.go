// Package rules implements the Phase N v3.x-2 layered-rules resolver:
// general → project → agent deep-merge with most-specific-wins semantics.
//
// Resolution model (per docs/plans/2026-05-07-phase-n-v3.x-1.5-agent-consumption-design-spike.md §2.1):
//
//   - Layer 1: ~/.bot-hq/rules/general.yaml  — cross-project trio rules
//   - Layer 2: ~/.bot-hq/projects/<p>.yaml   — project-specific overrides
//   - Layer 3: ~/.bot-hq/rules/agents/<a>.yaml — per-agent role + exec
//
// Conflict policy: leaf values from later layers replace earlier layers.
// Maps merge recursively (deep-merge); slices and scalars overwrite wholesale.
//
// Forward-compat: unknown keys preserved through the merge — schema validation
// is enforced by the webui write-handler (`internal/webui/rules_schema.go`),
// not the resolver. The resolver is intentionally schema-agnostic so new
// rule categories can land without resolver changes.
package rules

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"

	"gopkg.in/yaml.v3"
)

// Resolve returns the deep-merged rule-set for the given (project, agent)
// pair. Layers in resolution order: general → project → agent.
//
// Missing files are treated as empty layers (not errors) so the resolver
// works on partial substrates. Unrecoverable errors (yaml parse failure,
// permission denied) abort the resolution with a wrapped error.
//
// Agent layer is merged into the top-level result under key "agent" (so a
// caller can introspect role + exec without colliding with general/project
// keys that share the same shape). This matches the existing webui
// `resolveRules` behavior.
func Resolve(canonRoot, project, agent string) (map[string]any, error) {
	merged := map[string]any{}

	// Layer 1: general
	g, err := loadYAMLMap(filepath.Join(canonRoot, "rules", "general.yaml"))
	if err != nil {
		return nil, fmt.Errorf("general: %w", err)
	}
	merged = DeepMerge(merged, g)

	// Layer 2: project (looks at projects/<p>.yaml first; falls back to
	// rules/projects/<p>.yaml for the README-claimed location).
	if project != "" {
		paths := []string{
			filepath.Join(canonRoot, "projects", project+".yaml"),
			filepath.Join(canonRoot, "rules", "projects", project+".yaml"),
		}
		for _, path := range paths {
			pm, err := loadYAMLMap(path)
			if err != nil {
				return nil, fmt.Errorf("project %s: %w", project, err)
			}
			if pm != nil {
				merged = DeepMerge(merged, pm)
				break // first hit wins; don't double-merge
			}
		}
	}

	// Layer 3: agent (separate key per webui convention)
	if agent != "" {
		am, err := loadYAMLMap(filepath.Join(canonRoot, "rules", "agents", agent+".yaml"))
		if err != nil {
			return nil, fmt.Errorf("agent %s: %w", agent, err)
		}
		if am != nil {
			merged["agent"] = am
		}
	}
	return merged, nil
}

// ResolveMaps performs the same deep-merge but accepts pre-loaded maps for
// each layer. Useful for tests and for callers that have already parsed
// YAML (e.g., the webui handler reusing its own load logic).
//
// Conflict policy matches Resolve: leaf-replace; map-recurse. Agent merges
// under the "agent" top-level key.
func ResolveMaps(general, project, agent map[string]any) map[string]any {
	merged := DeepMerge(map[string]any{}, general)
	merged = DeepMerge(merged, project)
	if len(agent) > 0 {
		merged["agent"] = agent
	}
	return merged
}

// DeepMerge recursively merges over into base; over keys win on conflict.
// Maps merge recursively; slices and scalars overwrite wholesale (no
// element-wise list merging — too lossy for rule-sets where order matters).
//
// Returns a fresh map (does not mutate inputs).
func DeepMerge(base, over map[string]any) map[string]any {
	out := make(map[string]any, len(base)+len(over))
	for k, v := range base {
		out[k] = v
	}
	for k, ov := range over {
		if bv, has := out[k]; has {
			bm, bok := bv.(map[string]any)
			om, ook := ov.(map[string]any)
			if bok && ook {
				out[k] = DeepMerge(bm, om)
				continue
			}
		}
		out[k] = ov
	}
	return out
}

// loadYAMLMap reads a YAML file as a generic map[string]any. Missing file
// returns (nil, nil) so the resolver can treat absent layers as empty.
func loadYAMLMap(path string) (map[string]any, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		if errors.Is(err, os.ErrNotExist) {
			return nil, nil
		}
		return nil, err
	}
	var m map[string]any
	if err := yaml.Unmarshal(data, &m); err != nil {
		return nil, fmt.Errorf("parse %s: %w", path, err)
	}
	return m, nil
}
