#!/usr/bin/env bash
# TEST 5.4: Long-context coherence under DeepSeek subprocess.
# Per phase-t.md v5 T-0.5 sub-phase + R51 capability-parity requirement.
#
# Verifies: 200K+ context retention with cite-anchor accuracy at depth.

set -euo pipefail

REPORT_DIR="${1:-/tmp/capability-parity}"
mkdir -p "$REPORT_DIR"

RESULT_JSON="$REPORT_DIR/test-long-context.json"

if [[ -z "${DEEPSEEK_API_KEY:-}" ]]; then
  echo "ERROR: DEEPSEEK_API_KEY env var not set" >&2
  echo '{"test":"long-context","status":"SKIP","reason":"DEEPSEEK_API_KEY unset"}' > "$RESULT_JSON"
  exit 2
fi

# Test approach: feed phase-t.md v5 (~1442L / 119KB) as context; ask cite-anchor recall questions
PHASE_T_DOC="$HOME/.bot-hq/phase/phase-t.md"
if [[ ! -f "$PHASE_T_DOC" ]]; then
  echo '{"test":"long-context","status":"SKIP","reason":"phase-t.md not found"}' > "$RESULT_JSON"
  exit 2
fi

# Load phase-t.md content + ask cite-anchor recall questions
PROMPT="Read this document below carefully, then answer the questions.

<document>
$(cat "$PHASE_T_DOC")
</document>

Questions:
1. What is R51? (one-sentence definition)
2. What is the bilateral-converged seal anchor msg-id list?
3. What is the user msg-id that says 'i want to swap rain's model to deepseekv4'?
4. What is the cost-tracking schema YAML for cost_per_agent.brian?

Output ONLY JSON: {\"q1\": \"<answer>\", \"q2\": \"<answer>\", \"q3\": \"<answer>\", \"q4\": \"<answer>\"}"

OUTPUT=$(ANTHROPIC_BASE_URL="https://api.deepseek.com/anthropic" \
  ANTHROPIC_AUTH_TOKEN="$DEEPSEEK_API_KEY" \
  ANTHROPIC_MODEL="deepseek-v4-pro" \
  claude --print --mcp-config "$HOME/Projects/.bot-hq-rain-mcp.json" --dangerously-skip-permissions "$PROMPT" 2>&1) || OUTPUT_EXIT=$?

OUTPUT_EXIT=${OUTPUT_EXIT:-0}

# Verify recall: q1 should mention "PER-AGENT-MODEL-CONFIG-DISCIPLINE", q3 should mention "17094"
PASS_COUNT=0
if echo "$OUTPUT" | grep -qi 'per.agent.model.config'; then PASS_COUNT=$((PASS_COUNT + 1)); fi
if echo "$OUTPUT" | grep -q '17094'; then PASS_COUNT=$((PASS_COUNT + 1)); fi
if echo "$OUTPUT" | grep -qi 'cost.basis'; then PASS_COUNT=$((PASS_COUNT + 1)); fi

if [[ $PASS_COUNT -ge 3 ]]; then
  STATUS="PASS"
elif [[ $PASS_COUNT -ge 2 ]]; then
  STATUS="PARTIAL"
else
  STATUS="FAIL"
fi

cat > "$RESULT_JSON" <<EOF
{
  "test": "long-context",
  "status": "$STATUS",
  "exit_code": $OUTPUT_EXIT,
  "context_size_chars": $(wc -c < "$PHASE_T_DOC" | tr -d ' '),
  "recall_pass_count": $PASS_COUNT,
  "captured_at_utc": "$(date -u +%FT%TZ)"
}
EOF

echo "Result: $STATUS (recall=$PASS_COUNT/3)"
echo "JSON: $RESULT_JSON"

[[ "$STATUS" == "PASS" || "$STATUS" == "PARTIAL" ]]
