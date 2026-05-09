#!/usr/bin/env bash
# TEST 5.1: MCP tool-discovery + invocation under DeepSeek subprocess.
# Per phase-t.md v5 T-0.5 sub-phase + R51 capability-parity requirement.
#
# Verifies: Rain-DeepSeek subprocess can discover + invoke MCP tools
# (mcp__bot-hq__hub_send / hub_read / hub_status / etc.).

set -euo pipefail

REPORT_DIR="${1:-/tmp/capability-parity}"
mkdir -p "$REPORT_DIR"

RESULT_JSON="$REPORT_DIR/test-mcp-discovery.json"

if [[ -z "${DEEPSEEK_API_KEY:-}" ]]; then
  echo "ERROR: DEEPSEEK_API_KEY env var not set" >&2
  echo '{"test":"mcp-discovery","status":"SKIP","reason":"DEEPSEEK_API_KEY unset"}' > "$RESULT_JSON"
  exit 2
fi

# Spawn claude CLI subprocess with DeepSeek backend + MCP config
# Test prompt: ask Rain-DeepSeek to invoke hub_status MCP tool

PROMPT='List all bot-hq MCP tools available to you. Then invoke mcp__bot-hq__hub_status to report agent count. Output ONLY JSON: {"tools_count": <int>, "agents_online": <int>, "tool_invocation": "success" | "failure"}'

OUTPUT=$(ANTHROPIC_BASE_URL="https://api.deepseek.com/anthropic" \
  ANTHROPIC_AUTH_TOKEN="$DEEPSEEK_API_KEY" \
  ANTHROPIC_MODEL="deepseek-v4-pro" \
  claude --print --mcp-config "$HOME/Projects/.bot-hq-rain-mcp.json" --dangerously-skip-permissions "$PROMPT" 2>&1) || OUTPUT_EXIT=$?

OUTPUT_EXIT=${OUTPUT_EXIT:-0}

# Parse output: expect JSON with tool_invocation=success
if echo "$OUTPUT" | grep -q '"tool_invocation": "success"'; then
  STATUS="PASS"
elif echo "$OUTPUT" | grep -q '"tool_invocation": "failure"'; then
  STATUS="FAIL"
else
  STATUS="UNCERTAIN"
fi

cat > "$RESULT_JSON" <<EOF
{
  "test": "mcp-discovery",
  "status": "$STATUS",
  "exit_code": $OUTPUT_EXIT,
  "raw_output_chars": $(echo -n "$OUTPUT" | wc -c | tr -d ' '),
  "captured_at_utc": "$(date -u +%FT%TZ)"
}
EOF

echo "Result: $STATUS (exit=$OUTPUT_EXIT)"
echo "JSON: $RESULT_JSON"

[[ "$STATUS" == "PASS" ]]
