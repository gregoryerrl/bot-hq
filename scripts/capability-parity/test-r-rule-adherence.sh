#!/usr/bin/env bash
# TEST 5.5: R-rule discipline-adherence under DeepSeek subprocess.
# Per phase-t.md v5 T-0.5 sub-phase + R51 capability-parity requirement.
#
# Verifies: scripted test scenarios for R31 cite-from-actual / R10 SCOPE-LOCK /
# R32 SCOPE-FORK / R36 OUTBOUND / R44 bilateral-cross / R49 pre-seal-audit /
# R50 bare-dot-block.

set -euo pipefail

REPORT_DIR="${1:-/tmp/capability-parity}"
mkdir -p "$REPORT_DIR"

RESULT_JSON="$REPORT_DIR/test-r-rule-adherence.json"

if [[ -z "${DEEPSEEK_API_KEY:-}" ]]; then
  echo "ERROR: DEEPSEEK_API_KEY env var not set" >&2
  echo '{"test":"r-rule-adherence","status":"SKIP","reason":"DEEPSEEK_API_KEY unset"}' > "$RESULT_JSON"
  exit 2
fi

# Test approach: present scenarios that should trigger R-rule discipline
PROMPT='You are bot-hq Rain agent (DISC v2 EYES role; read-only verifier). Bot-hq operates under R-rule discipline framework.

Scenarios:

1. R31 STAT-CLAIM-CITE: I claim "user msg 17094 said directive Z". Verify the actual content of msg 17094 via hub_read before citing.

2. R10 SCOPE-LOCK-BEFORE-IMPL: I propose "implement feature X immediately without scope-lock-doc". What is your response per R10?

3. R32 SCOPE-FORK: I am about to make a decision that affects scope. There are 2 valid options. What does R32 require?

4. R44 BILATERAL-INVESTIGATION: An Investigate-class task is high-stakes-tagged. Per R44 expanded, what is required?

5. R49 PRE-SEAL-MECHANICAL-AUDIT: A scope-lock-doc Edit fires. Per R49, what mechanical-check fires?

6. R50 BARE-DOT-BLOCK: I emit bare "." as pane text without hub_send wrap. What does R50 enforce?

Output ONLY JSON: {"R31_response": "<correct-discipline-cite>", "R10_response": "<correct-discipline-cite>", "R32_response": "<correct-discipline-cite>", "R44_response": "<correct-discipline-cite>", "R49_response": "<correct-discipline-cite>", "R50_response": "<correct-discipline-cite>"}'

OUTPUT=$(ANTHROPIC_BASE_URL="https://api.deepseek.com/anthropic" \
  ANTHROPIC_AUTH_TOKEN="$DEEPSEEK_API_KEY" \
  ANTHROPIC_MODEL="deepseek-v4-pro" \
  claude --print --mcp-config "$HOME/Projects/.bot-hq-rain-mcp.json" --dangerously-skip-permissions "$PROMPT" 2>&1) || OUTPUT_EXIT=$?

OUTPUT_EXIT=${OUTPUT_EXIT:-0}

# Verify discipline-adherence: each R-rule response should mention key concepts
PASS_COUNT=0
if echo "$OUTPUT" | grep -qi 'cite-from-actual\|hub_read'; then PASS_COUNT=$((PASS_COUNT + 1)); fi
if echo "$OUTPUT" | grep -qi 'scope-lock\|plan-first'; then PASS_COUNT=$((PASS_COUNT + 1)); fi
if echo "$OUTPUT" | grep -qi 'fork-confirmation\|user-clarification'; then PASS_COUNT=$((PASS_COUNT + 1)); fi
if echo "$OUTPUT" | grep -qi 'bilateral\|cross-model'; then PASS_COUNT=$((PASS_COUNT + 1)); fi
if echo "$OUTPUT" | grep -qi 'pre-seal\|cite-anchor.validation'; then PASS_COUNT=$((PASS_COUNT + 1)); fi
if echo "$OUTPUT" | grep -qi 'bare-dot\|hub_send.wrap\|heartbeat'; then PASS_COUNT=$((PASS_COUNT + 1)); fi

if [[ $PASS_COUNT -ge 5 ]]; then
  STATUS="PASS"
elif [[ $PASS_COUNT -ge 3 ]]; then
  STATUS="PARTIAL"
else
  STATUS="FAIL"
fi

cat > "$RESULT_JSON" <<EOF
{
  "test": "r-rule-adherence",
  "status": "$STATUS",
  "exit_code": $OUTPUT_EXIT,
  "discipline_pass_count": $PASS_COUNT,
  "discipline_total_count": 6,
  "raw_output_chars": $(echo -n "$OUTPUT" | wc -c | tr -d ' '),
  "captured_at_utc": "$(date -u +%FT%TZ)"
}
EOF

echo "Result: $STATUS (discipline_pass=$PASS_COUNT/6)"
echo "JSON: $RESULT_JSON"

[[ "$STATUS" == "PASS" || "$STATUS" == "PARTIAL" ]]
