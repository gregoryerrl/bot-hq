#!/usr/bin/env bash
# TEST 5.3: Subagent-dispatch under DeepSeek subprocess.
# Per phase-t.md v5 T-0.5 sub-phase + R51 capability-parity requirement.
#
# Verifies: Rain-DeepSeek subprocess can dispatch subagent via Agent tool
# with subagent_type=Explore / Plan / general-purpose.

set -euo pipefail

REPORT_DIR="${1:-/tmp/capability-parity}"
mkdir -p "$REPORT_DIR"

RESULT_JSON="$REPORT_DIR/test-subagent-dispatch.json"

if [[ -z "${DEEPSEEK_API_KEY:-}" ]]; then
  echo "ERROR: DEEPSEEK_API_KEY env var not set" >&2
  echo '{"test":"subagent-dispatch","status":"SKIP","reason":"DEEPSEEK_API_KEY unset"}' > "$RESULT_JSON"
  exit 2
fi

PROMPT='Dispatch a subagent (Agent tool with subagent_type=general-purpose) to read /etc/hostname and report its content. Wait for subagent return. Output ONLY JSON: {"subagent_dispatched": "yes" | "no", "subagent_returned": "yes" | "no", "result_received": "<hostname-or-empty>"}'

OUTPUT=$(ANTHROPIC_BASE_URL="https://api.deepseek.com/anthropic" \
  ANTHROPIC_AUTH_TOKEN="$DEEPSEEK_API_KEY" \
  ANTHROPIC_MODEL="deepseek-v4-pro" \
  claude --print --mcp-config "$HOME/Projects/.bot-hq-rain-mcp.json" --dangerously-skip-permissions "$PROMPT" 2>&1) || OUTPUT_EXIT=$?

OUTPUT_EXIT=${OUTPUT_EXIT:-0}

if echo "$OUTPUT" | grep -q '"subagent_dispatched": "yes"' && echo "$OUTPUT" | grep -q '"subagent_returned": "yes"'; then
  STATUS="PASS"
elif echo "$OUTPUT" | grep -q '"subagent_dispatched": "yes"'; then
  STATUS="PARTIAL"
else
  STATUS="FAIL"
fi

cat > "$RESULT_JSON" <<EOF
{
  "test": "subagent-dispatch",
  "status": "$STATUS",
  "exit_code": $OUTPUT_EXIT,
  "raw_output_chars": $(echo -n "$OUTPUT" | wc -c | tr -d ' '),
  "captured_at_utc": "$(date -u +%FT%TZ)"
}
EOF

echo "Result: $STATUS (exit=$OUTPUT_EXIT)"
echo "JSON: $RESULT_JSON"

[[ "$STATUS" == "PASS" ]]
