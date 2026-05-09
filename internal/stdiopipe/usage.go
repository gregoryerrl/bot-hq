// Package stdiopipe — usage.go: T-2.7 LLM-call-site cost-tracking helpers.
//
// Parses claude CLI subprocess output (--output-format stream-json) for
// per-call token usage + computes USD cost via per-provider/model price
// table. Wires Driver.Wait() → IPIVRuntime.RecordPhaseUsage downstream.
//
// Public API:
//   - Usage struct: token counts + computed cost
//   - ParseUsage(output): scan stream-json output for assistant-message usage
//   - ComputeCost(provider, model, in, out): USD cost from price table
//
// Per phase-t.md v5 T-5 + T-2.7: cost-tracking is foundational for the
// per-phase compute-budget allocator + circuit-breaker (warn-80%/block-100%).

package stdiopipe

import (
	"encoding/json"
	"fmt"
	"strings"
)

// Usage captures token + cost data from a single LLM subprocess invocation.
// Populated by ParseUsage() on subprocess output and CostUSD computed via
// ComputeCost() at parse-completion.
type Usage struct {
	InputTokens  int
	OutputTokens int
	Model        string
	CostUSD      float64
}

// pricePerTokenUSD maps "<provider>|<model>" → input/output USD-per-1K-tokens.
// Source: vendor pricing pages as of 2026-05-09.
//   - Anthropic Claude Opus 4.7:  $15/1M input  / $75/1M  output
//   - Anthropic Claude Sonnet 4.6: $3/1M input  / $15/1M  output
//   - DeepSeek-V4-Pro:            $0.27/1M input / $1.10/1M output
//
// Models not in the table return CostUSD=0 (subscription paths or
// unrecognized models — caller treats 0 as "not tracked at this site").
var pricePerTokenUSD = map[string]struct {
	InputUSDPer1K  float64
	OutputUSDPer1K float64
}{
	"anthropic|claude-default":     {InputUSDPer1K: 0.015, OutputUSDPer1K: 0.075},
	"anthropic|claude-opus-4-7":    {InputUSDPer1K: 0.015, OutputUSDPer1K: 0.075},
	"anthropic|claude-sonnet-4-6":  {InputUSDPer1K: 0.003, OutputUSDPer1K: 0.015},
	"anthropic|claude-haiku-4-5":   {InputUSDPer1K: 0.001, OutputUSDPer1K: 0.005},
	"deepseek|deepseek-v4-pro":     {InputUSDPer1K: 0.00027, OutputUSDPer1K: 0.0011},
}

// ComputeCost returns USD cost for the given provider+model+token counts.
// Returns 0 when the (provider, model) pair is not in the price table.
func ComputeCost(provider, model string, inputTokens, outputTokens int) float64 {
	key := provider + "|" + model
	p, ok := pricePerTokenUSD[key]
	if !ok {
		return 0
	}
	return (float64(inputTokens)*p.InputUSDPer1K)/1000.0 +
		(float64(outputTokens)*p.OutputUSDPer1K)/1000.0
}

// ParseUsage scans claude --output-format stream-json output (one JSON
// envelope per line) and aggregates token usage across all "assistant"
// envelopes. Returns nil + nil-error when no usage data is present
// (caller decides whether absence is an error). Returns non-nil error
// only on syntactically-malformed JSON.
func ParseUsage(output string) (*Usage, error) {
	var totalInput, totalOutput int
	var foundAny bool
	var modelName string
	for _, line := range strings.Split(output, "\n") {
		line = strings.TrimSpace(line)
		if line == "" || !strings.HasPrefix(line, "{") {
			continue
		}
		var env streamJSONEnvelope
		if err := json.Unmarshal([]byte(line), &env); err != nil {
			return nil, fmt.Errorf("parse stream-json line: %w", err)
		}
		if env.Type != "assistant" || env.Message == nil {
			continue
		}
		if env.Message.Usage != nil {
			totalInput += env.Message.Usage.InputTokens
			totalOutput += env.Message.Usage.OutputTokens
			foundAny = true
		}
		if env.Message.Model != "" && modelName == "" {
			modelName = env.Message.Model
		}
	}
	if !foundAny {
		return nil, nil
	}
	return &Usage{
		InputTokens:  totalInput,
		OutputTokens: totalOutput,
		Model:        modelName,
	}, nil
}

type streamJSONEnvelope struct {
	Type    string             `json:"type"`
	Message *streamJSONMessage `json:"message,omitempty"`
}

type streamJSONMessage struct {
	Model string           `json:"model,omitempty"`
	Usage *streamJSONUsage `json:"usage,omitempty"`
}

type streamJSONUsage struct {
	InputTokens  int `json:"input_tokens"`
	OutputTokens int `json:"output_tokens"`
}
