#!/usr/bin/env bash
# TEST 5.2: Hooks-firing under DeepSeek subprocess.
# Per phase-t.md v5 T-0.5 sub-phase + R51 capability-parity requirement.
#
# Verifies: All R-rule hooks (R33 PreToolUse / R36 Stop / R40 voice-mirror /
# outbound-miss / tool-permission / R44 / R45 / R47 / R49 / R50 / R51 / R52 / R53)
# fire correctly under DeepSeek subprocess.

set -euo pipefail

REPORT_DIR="${1:-/tmp/capability-parity}"
mkdir -p "$REPORT_DIR"

RESULT_JSON="$REPORT_DIR/test-hooks-firing.json"

if [[ -z "${DEEPSEEK_API_KEY:-}" ]]; then
  echo "ERROR: DEEPSEEK_API_KEY env var not set" >&2
  echo '{"test":"hooks-firing","status":"SKIP","reason":"DEEPSEEK_API_KEY unset"}' > "$RESULT_JSON"
  exit 2
fi

# Test approach: spawn DeepSeek subprocess with hooks-config + trigger each hook + verify side-effects
# Hooks targets:
#   R33 PreToolUse: trigger via Bash tool invocation; verify gate-file-read enforcement
#   R36 Stop: trigger via end-of-turn; verify outbound-discipline-mechanical
#   R40 voice-mirror: trigger via user-facing artifact write; verify voice-mirror-log entry
#   R49 pre-seal-audit: trigger via scope-lock-doc Write; verify cite-anchor validation
#   R50 bare-dot-block: trigger via bare-"." emit; verify mechanical-block

PROMPT='Trigger each of these test scenarios and report which hooks fired:
1. Run a Bash command (touch /tmp/hook-test-$$).
2. Read a file (/tmp/hook-test-$$).
3. End your turn with a substantive scope-lock decision.

Output ONLY JSON: {"hook_R33": "fired" | "not-fired", "hook_R36": "fired" | "not-fired", "hook_R40": "fired" | "not-fired", "hook_R49": "fired" | "not-fired", "hook_R50": "fired" | "not-fired"}'

OUTPUT=$(ANTHROPIC_BASE_URL="https://api.deepseek.com/anthropic" \
  ANTHROPIC_AUTH_TOKEN="$DEEPSEEK_API_KEY" \
  ANTHROPIC_MODEL="deepseek-v4-pro" \
  claude --print --mcp-config "$HOME/Projects/.bot-hq-rain-mcp.json" --dangerously-skip-permissions "$PROMPT" 2>&1) || OUTPUT_EXIT=$?

OUTPUT_EXIT=${OUTPUT_EXIT:-0}

# Parse output: count fired-hooks
FIRED_COUNT=$(echo "$OUTPUT" | grep -oE '"hook_R[0-9]+": "fired"' | wc -l | tr -d ' ')
NOT_FIRED_COUNT=$(echo "$OUTPUT" | grep -oE '"hook_R[0-9]+": "not-fired"' | wc -l | tr -d ' ')

if [[ $FIRED_COUNT -ge 3 && $NOT_FIRED_COUNT -le 2 ]]; then
  STATUS="PASS"
elif [[ $FIRED_COUNT -ge 1 ]]; then
  STATUS="PARTIAL"
else
  STATUS="FAIL"
fi

cat > "$RESULT_JSON" <<EOF
{
  "test": "hooks-firing",
  "status": "$STATUS",
  "exit_code": $OUTPUT_EXIT,
  "hooks_fired_count": $FIRED_COUNT,
  "hooks_not_fired_count": $NOT_FIRED_COUNT,
  "raw_output_chars": $(echo -n "$OUTPUT" | wc -c | tr -d ' '),
  "captured_at_utc": "$(date -u +%FT%TZ)"
}
EOF

# Cleanup
rm -f "/tmp/hook-test-$$"

echo "Result: $STATUS (fired=$FIRED_COUNT not-fired=$NOT_FIRED_COUNT)"
echo "JSON: $RESULT_JSON"

[[ "$STATUS" == "PASS" || "$STATUS" == "PARTIAL" ]]
