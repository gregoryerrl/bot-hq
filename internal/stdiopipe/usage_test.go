package stdiopipe

import (
	"math"
	"strings"
	"testing"
)

// nearlyEqual returns true when |a-b| < 1e-9 — sufficient for cost-tracking
// price-table multiplications (smallest unit: $0.00027/1K-input-tokens).
func nearlyEqual(a, b float64) bool {
	return math.Abs(a-b) < 1e-9
}

// ====== ComputeCost ======

func TestComputeCost_anthropicClaudeOpus(t *testing.T) {
	// 1000 input + 500 output → 1*0.015 + 0.5*0.075 = 0.015 + 0.0375 = 0.0525
	got := ComputeCost("anthropic", "claude-opus-4-7", 1000, 500)
	want := 0.0525
	if !nearlyEqual(got, want) {
		t.Errorf("ComputeCost claude-opus-4-7 = %.9f, want %.9f", got, want)
	}
}

func TestComputeCost_anthropicClaudeDefault(t *testing.T) {
	// claude-default = same prices as opus-4-7
	got := ComputeCost("anthropic", "claude-default", 2000, 1000)
	want := 0.105 // 2*0.015 + 1*0.075
	if !nearlyEqual(got, want) {
		t.Errorf("ComputeCost claude-default = %.9f, want %.9f", got, want)
	}
}

func TestComputeCost_anthropicSonnet(t *testing.T) {
	// 5000 input + 2000 output → 5*0.003 + 2*0.015 = 0.015 + 0.030 = 0.045
	got := ComputeCost("anthropic", "claude-sonnet-4-6", 5000, 2000)
	want := 0.045
	if !nearlyEqual(got, want) {
		t.Errorf("ComputeCost claude-sonnet-4-6 = %.9f, want %.9f", got, want)
	}
}

func TestComputeCost_deepseekV4Pro(t *testing.T) {
	// 10000 input + 5000 output → 10*0.00027 + 5*0.0011 = 0.0027 + 0.0055 = 0.0082
	got := ComputeCost("deepseek", "deepseek-v4-pro", 10000, 5000)
	want := 0.0082
	if !nearlyEqual(got, want) {
		t.Errorf("ComputeCost deepseek-v4-pro = %.9f, want %.9f", got, want)
	}
}

func TestComputeCost_openaiGPT4Turbo(t *testing.T) {
	// 1000 input + 500 output → 1*0.01 + 0.5*0.03 = 0.025
	got := ComputeCost("openai", "gpt-4-turbo", 1000, 500)
	want := 0.025
	if !nearlyEqual(got, want) {
		t.Errorf("ComputeCost openai|gpt-4-turbo = %.9f, want %.9f", got, want)
	}
}

func TestComputeCost_googleGeminiPro(t *testing.T) {
	// 2000 input + 1000 output → 2*0.0035 + 1*0.0105 = 0.0175
	got := ComputeCost("google-vertex", "gemini-1.5-pro", 2000, 1000)
	want := 0.0175
	if !nearlyEqual(got, want) {
		t.Errorf("ComputeCost google-vertex|gemini-1.5-pro = %.9f, want %.9f", got, want)
	}
}

func TestComputeCost_googleGeminiFlash(t *testing.T) {
	// 100000 input + 50000 output → 100*0.000075 + 50*0.0003 = 0.0075 + 0.015 = 0.0225
	got := ComputeCost("google-vertex", "gemini-1.5-flash", 100000, 50000)
	want := 0.0225
	if !nearlyEqual(got, want) {
		t.Errorf("ComputeCost google-vertex|gemini-1.5-flash = %.9f, want %.9f", got, want)
	}
}

func TestComputeCost_mistralLarge(t *testing.T) {
	// 5000 input + 2500 output → 5*0.004 + 2.5*0.012 = 0.020 + 0.030 = 0.050
	got := ComputeCost("mistral", "mistral-large-latest", 5000, 2500)
	want := 0.050
	if !nearlyEqual(got, want) {
		t.Errorf("ComputeCost mistral|mistral-large-latest = %.9f, want %.9f", got, want)
	}
}

func TestComputeCost_unknownModelReturnsZero(t *testing.T) {
	if got := ComputeCost("unknown-provider", "unknown-model", 1000, 500); got != 0 {
		t.Errorf("ComputeCost unknown = %.6f, want 0 (not in price table)", got)
	}
	if got := ComputeCost("anthropic", "claude-bogus-model", 1000, 500); got != 0 {
		t.Errorf("ComputeCost anthropic|bogus = %.6f, want 0", got)
	}
}

func TestComputeCost_zeroTokens(t *testing.T) {
	if got := ComputeCost("anthropic", "claude-opus-4-7", 0, 0); got != 0 {
		t.Errorf("ComputeCost zero-tokens = %.6f, want 0", got)
	}
}

// ====== ParseUsage ======

func TestParseUsage_singleAssistantLineWithUsage(t *testing.T) {
	output := `{"type":"system","subtype":"init","session_id":"abc"}
{"type":"user","message":{"content":"hello"}}
{"type":"assistant","message":{"model":"claude-opus-4-7","content":[{"type":"text","text":"hi"}],"usage":{"input_tokens":10,"output_tokens":20}}}
{"type":"result","subtype":"success","is_error":false}`

	usage, err := ParseUsage(output)
	if err != nil {
		t.Fatalf("ParseUsage error: %v", err)
	}
	if usage == nil {
		t.Fatal("usage is nil")
	}
	if usage.InputTokens != 10 {
		t.Errorf("InputTokens = %d, want 10", usage.InputTokens)
	}
	if usage.OutputTokens != 20 {
		t.Errorf("OutputTokens = %d, want 20", usage.OutputTokens)
	}
	if usage.Model != "claude-opus-4-7" {
		t.Errorf("Model = %q, want claude-opus-4-7", usage.Model)
	}
}

func TestParseUsage_multipleAssistantLinesAggregated(t *testing.T) {
	output := `{"type":"assistant","message":{"model":"claude-opus-4-7","usage":{"input_tokens":100,"output_tokens":50}}}
{"type":"assistant","message":{"model":"claude-opus-4-7","usage":{"input_tokens":200,"output_tokens":80}}}
{"type":"result"}`

	usage, err := ParseUsage(output)
	if err != nil {
		t.Fatalf("ParseUsage error: %v", err)
	}
	if usage.InputTokens != 300 {
		t.Errorf("aggregated InputTokens = %d, want 300", usage.InputTokens)
	}
	if usage.OutputTokens != 130 {
		t.Errorf("aggregated OutputTokens = %d, want 130", usage.OutputTokens)
	}
}

func TestParseUsage_noAssistantLinesReturnsNilNoError(t *testing.T) {
	output := `{"type":"system","subtype":"init"}
{"type":"user","message":{"content":"hi"}}
{"type":"result","subtype":"success"}`

	usage, err := ParseUsage(output)
	if err != nil {
		t.Fatalf("ParseUsage error: %v", err)
	}
	if usage != nil {
		t.Errorf("usage = %+v, want nil for no-assistant-lines", usage)
	}
}

func TestParseUsage_emptyOutputReturnsNilNoError(t *testing.T) {
	usage, err := ParseUsage("")
	if err != nil {
		t.Fatalf("ParseUsage error on empty: %v", err)
	}
	if usage != nil {
		t.Errorf("usage = %+v, want nil for empty output", usage)
	}
}

func TestParseUsage_skipsNonJSONLines(t *testing.T) {
	output := `not json at all
random log noise
{"type":"assistant","message":{"model":"x","usage":{"input_tokens":5,"output_tokens":7}}}
trailing noise`

	usage, err := ParseUsage(output)
	if err != nil {
		t.Fatalf("ParseUsage should skip non-JSON: %v", err)
	}
	if usage == nil || usage.InputTokens != 5 || usage.OutputTokens != 7 {
		t.Errorf("usage = %+v, want {5,7}", usage)
	}
}

func TestParseUsage_malformedJSONReturnsError(t *testing.T) {
	output := `{"type":"assistant","message":{"usage":{"input_tokens":INVALID}}}`
	_, err := ParseUsage(output)
	if err == nil {
		t.Fatal("expected error for malformed JSON")
	}
	if !strings.Contains(err.Error(), "parse stream-json") {
		t.Errorf("error %q does not mention parse-stream-json", err)
	}
}

func TestParseUsage_assistantWithoutUsageBlock(t *testing.T) {
	// "assistant" envelope without usage field — should not count
	output := `{"type":"assistant","message":{"model":"x","content":[{"type":"text","text":"hi"}]}}
{"type":"result"}`
	usage, err := ParseUsage(output)
	if err != nil {
		t.Fatalf("ParseUsage error: %v", err)
	}
	if usage != nil {
		t.Errorf("usage = %+v, want nil when no usage block present", usage)
	}
}

func TestParseUsage_modelTakenFromFirstAssistant(t *testing.T) {
	// when multiple assistant lines have different models, first wins
	output := `{"type":"assistant","message":{"model":"first-model","usage":{"input_tokens":1,"output_tokens":2}}}
{"type":"assistant","message":{"model":"second-model","usage":{"input_tokens":3,"output_tokens":4}}}`
	usage, err := ParseUsage(output)
	if err != nil {
		t.Fatalf("ParseUsage error: %v", err)
	}
	if usage.Model != "first-model" {
		t.Errorf("Model = %q, want first-model", usage.Model)
	}
	if usage.InputTokens != 4 || usage.OutputTokens != 6 {
		t.Errorf("aggregated tokens = (%d, %d), want (4, 6)", usage.InputTokens, usage.OutputTokens)
	}
}

// ====== Driver.LastUsage integration ======

func TestDriver_lastUsageNilBeforeWait(t *testing.T) {
	db := newTestDB(t)
	d, err := New(db, "rain")
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	if d.LastUsage() != nil {
		t.Errorf("LastUsage = %+v, want nil before Wait()", d.LastUsage())
	}
}

func TestDriver_parseLastUsageNoOpOnEmptyCaptured(t *testing.T) {
	db := newTestDB(t)
	d, err := New(db, "rain")
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	d.parseLastUsage() // no captured output → no-op
	if d.LastUsage() != nil {
		t.Errorf("LastUsage = %+v, want nil with empty captured", d.LastUsage())
	}
}

func TestDriver_parseLastUsagePopulatesFromCaptured(t *testing.T) {
	db := newTestDB(t)
	d, err := New(db, "rain")
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	// Simulate Receive() having accumulated stream-json output
	d.captured.WriteString(`{"type":"assistant","message":{"model":"deepseek-v4-pro","usage":{"input_tokens":1000,"output_tokens":500}}}`)
	d.captured.WriteByte('\n')

	d.parseLastUsage()

	usage := d.LastUsage()
	if usage == nil {
		t.Fatal("LastUsage = nil after parseLastUsage")
	}
	if usage.InputTokens != 1000 || usage.OutputTokens != 500 {
		t.Errorf("usage tokens = (%d, %d), want (1000, 500)", usage.InputTokens, usage.OutputTokens)
	}
	// Rain config = deepseek + deepseek-v4-pro → CostUSD computed
	wantCost := (1000 * 0.00027 / 1000.0) + (500 * 0.0011 / 1000.0)
	if !nearlyEqual(usage.CostUSD, wantCost) {
		t.Errorf("CostUSD = %.9f, want %.9f", usage.CostUSD, wantCost)
	}
}

func TestDriver_parseLastUsageZeroCostForUnknownClaudeModel(t *testing.T) {
	db := newTestDB(t)
	d, err := New(db, "brian") // brian = anthropic claude-default
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	d.captured.WriteString(`{"type":"assistant","message":{"model":"claude-opus-4-7","usage":{"input_tokens":2000,"output_tokens":1000}}}`)
	d.captured.WriteByte('\n')

	d.parseLastUsage()

	usage := d.LastUsage()
	if usage == nil {
		t.Fatal("LastUsage nil")
	}
	// Brian config = anthropic|claude-default → in price table; cost computed
	wantCost := (2000 * 0.015 / 1000.0) + (1000 * 0.075 / 1000.0)
	if !nearlyEqual(usage.CostUSD, wantCost) {
		t.Errorf("CostUSD = %.9f, want %.9f", usage.CostUSD, wantCost)
	}
}
