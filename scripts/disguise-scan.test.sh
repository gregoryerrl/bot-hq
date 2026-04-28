#!/usr/bin/env bash
# disguise-scan.test.sh — smoke test for disguise-scan.sh.
#
# Phase I W1b G-4 ratchet. Validates positive (leak detected → exit 1),
# negative (clean → exit 0), and FP-class (Discord/Discovered must NOT
# trigger → exit 0) cases.
#
# Run from bot-hq repo root: ./scripts/disguise-scan.test.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCAN="$SCRIPT_DIR/disguise-scan.sh"

if [[ ! -x "$SCAN" ]]; then
    echo "FAIL: $SCAN is not executable (chmod +x)" >&2
    exit 1
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

pass=0
fail=0

# Run scan against a fixture file content; assert exit status matches expected.
# Args: <case-name> <expected-status> <fixture-content>
expect_status() {
    local name="$1" want="$2" content="$3"
    local f="$tmp/${name}.md"
    printf '%s\n' "$content" >"$f"
    local got=0
    if "$SCAN" "$f" >/dev/null 2>&1; then
        got=0
    else
        got=$?
    fi
    if [[ "$got" -eq "$want" ]]; then
        echo "PASS: $name (exit $got)"
        pass=$((pass + 1))
    else
        echo "FAIL: $name expected exit $want, got exit $got"
        echo "      content: $content"
        fail=$((fail + 1))
    fi
}

# === Positive cases (leak — should exit 1) ===
expect_status "lowercase_brian"     1 "Brian completed the audit yesterday."
expect_status "uppercase_BRAIN"     1 "Joint BRAIN scope-lock ratified the design."
expect_status "uppercase_HANDS"     1 "HANDS execute via Brian dispatch."
expect_status "uppercase_EYES"      1 "EYES verify before commit."
expect_status "lowercase_rain"      1 "Rain delivered the retro review."
expect_status "agent_emma"          1 "Emma flagged the always-flag pattern."
expect_status "agent_clive"         1 "Clive received the inbound message."
expect_status "hub_underscore"      1 "Use hub_send to route the response."
expect_status "hub_dash"            1 "Inter-hub-traffic counts as cross-coord."
expect_status "branch_pre_rain"     1 "Branch feature/pre-rain-review needs cleanup."
expect_status "branch_rain_slash"   1 "Path rain/checkpoint-2026-04-28.md is stale."
expect_status "msg_id_short"        1 "See msg #4242 for full context."
expect_status "message_id_long"     1 "Cross-references: message #12345 captures the decision."
expect_status "greenflag_token"     1 "Awaiting greenflag clock from Rain."
expect_status "trio_token"          1 "The trio coordinated via BRAIN-cycle."
expect_status "coder_dispatch"      1 "coder.dispatch returned PR #28."
expect_status "agent_id_phrase"     1 "Use the registered agent ID for routing."
expect_status "mcp_filename"        1 "Stale config: .bot-hq-coder-a1b2c3d4-mcp.json"
expect_status "disc_v2_token"       1 "DISC v2 audience-driven routing applies."
expect_status "bot_hq_word"         1 "The bot-hq trio handled the cycle."
expect_status "bot_dot_hq"          1 "Reference: bot.hq orchestrator session."

# === Negative cases (clean — should exit 0) ===
expect_status "discord_fp"          0 "Discord notification fired on schedule."
expect_status "discovered_fp"       0 "Discovered the bug via property test."
expect_status "discord_platform"    0 "The Discord platform integrates with Slack."
expect_status "clean_eng_doc"       0 "The Postgres connector handles SSL termination."
expect_status "disc_golf"           0 "DISC GOLF is a sport." # bare DISC without v2 — should not match
expect_status "regex_word"          0 "The grep -E flag enables extended regex."

# === Documented-FP cases (would-FP-but-accepted-trade) ===
# These are positive matches that catch agent references AND non-agent uses
# (e.g., \brain\b matches "weather rain"). The feedback-memory accepts this
# trade — review-and-override is cheaper than weakening the regex and missing
# real Rain/RAIN agent leaks. Tests pinned to exit 1 so changes to the regex
# that silently shrink coverage are caught here.
expect_status "rain_weather_fp"     1 "Forecast: heavy rain expected."

# === FP-class boundary cases ===
expect_status "Discord_capital"     0 "Discord (the platform)"
expect_status "RAINS_substring"     0 "It RAINS in autumn." # \brain\b word-boundary should not match RAINS

echo ""
echo "Results: $pass passed, $fail failed"
if [[ "$fail" -gt 0 ]]; then
    exit 1
fi
exit 0
