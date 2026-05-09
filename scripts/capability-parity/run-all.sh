#!/usr/bin/env bash
# Aggregator: runs all capability-parity TEST 5.x sub-tests sequentially.
# Per phase-t.md v5 T-0.5 sub-phase + R51 capability-parity requirement.
#
# Usage:
#   DEEPSEEK_API_KEY=<key> ./run-all.sh [--report-dir <path>]
#
# Outputs: per-test JSON results + aggregator pass/fail summary.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPORT_DIR="${1:-$HOME/.bot-hq/diag/capability-parity/$(date +%Y%m%d-%H%M%S)}"

mkdir -p "$REPORT_DIR"

if [[ -z "${DEEPSEEK_API_KEY:-}" ]]; then
  echo "ERROR: DEEPSEEK_API_KEY env var not set" >&2
  echo "  This script tests live DeepSeek-V4-Pro capability-parity per phase-t.md v5 R51." >&2
  echo "  Set DEEPSEEK_API_KEY (rotate first if leaked per security discipline)." >&2
  exit 2
fi

echo "=== capability-parity TEST 5.x runner ==="
echo "Report dir: $REPORT_DIR"
echo "Started: $(date -u +%FT%TZ)"
echo

TESTS=(
  "test-mcp-discovery.sh"
  "test-hooks-firing.sh"
  "test-subagent-dispatch.sh"
  "test-long-context.sh"
  "test-r-rule-adherence.sh"
)

PASS_COUNT=0
FAIL_COUNT=0
SKIP_COUNT=0

for test in "${TESTS[@]}"; do
  test_path="$SCRIPT_DIR/$test"
  if [[ ! -x "$test_path" ]]; then
    echo "[SKIP] $test (not found or not executable)"
    SKIP_COUNT=$((SKIP_COUNT + 1))
    continue
  fi

  echo "[RUN] $test"
  if "$test_path" "$REPORT_DIR" 2>&1 | tee "$REPORT_DIR/${test%.sh}.log"; then
    echo "[PASS] $test"
    PASS_COUNT=$((PASS_COUNT + 1))
  else
    echo "[FAIL] $test"
    FAIL_COUNT=$((FAIL_COUNT + 1))
  fi
  echo
done

echo "=== summary ==="
echo "PASS: $PASS_COUNT / FAIL: $FAIL_COUNT / SKIP: $SKIP_COUNT"
echo "Report dir: $REPORT_DIR"

if [[ $FAIL_COUNT -gt 0 ]]; then
  exit 1
fi
