#!/usr/bin/env bash
# Phase N v3.x-2 SessionStart hook.
#
# Fires at Claude Code session-init; emits a markdown system-prompt prepend
# bundling: project overview, last bootstrap, resolved rules, active tasks.
# The harness wraps the stdout as a system-prefix message (per Anthropic
# SessionStart hook spec) so the agent enters its first turn with full
# project context loaded.
#
# Resolution: BOT_HQ_PROJECT > cwd-inference > "bot-hq".
# Agent: BOT_HQ_AGENT > "brian".
#
# Output also tee'd to ~/.bot-hq/sessions/<session-id>/session-open.md for
# audit (so we can verify what the agent saw).
#
# Install: copy to ~/.bot-hq/scripts/sessionstart-hook.sh + register in
# ~/.claude/settings.json under hooks.SessionStart. Or invoke directly:
#   bash ~/Projects/bot-hq/scripts/sessionstart-hook.sh

set -e

PROJECT="${BOT_HQ_PROJECT:-}"
AGENT="${BOT_HQ_AGENT:-brian}"
SESSION_ID="${CLAUDE_SESSION_ID:-$(date +%Y%m%d-%H%M%S)}"

# Locate bot-hq binary; fall back to in-flight build via `go run`.
BOTHQ_BIN="${BOT_HQ_BIN:-bot-hq}"
if ! command -v "$BOTHQ_BIN" >/dev/null 2>&1; then
    if [ -d "$HOME/Projects/bot-hq" ]; then
        BOTHQ_BIN="go run github.com/gregoryerrl/bot-hq/cmd/bot-hq"
        cd "$HOME/Projects/bot-hq" || exit 0
    else
        echo "<!-- bot-hq SessionStart hook: bot-hq binary not found -->"
        exit 0
    fi
fi

ARGS=()
if [ -n "$PROJECT" ]; then
    ARGS+=(--project "$PROJECT")
fi
if [ -n "$AGENT" ]; then
    ARGS+=(--agent "$AGENT")
fi

OUT="$($BOTHQ_BIN session-open "${ARGS[@]}" 2>/dev/null || true)"
if [ -z "$OUT" ]; then
    echo "<!-- bot-hq SessionStart hook: empty payload (daemon unreachable + build failed) -->"
    exit 0
fi

# Emit to stdout for harness injection.
echo "$OUT"

# Audit: tee to ~/.bot-hq/sessions/<id>/session-open.md.
AUDIT_DIR="$HOME/.bot-hq/sessions/$SESSION_ID"
mkdir -p "$AUDIT_DIR" 2>/dev/null || true
echo "$OUT" > "$AUDIT_DIR/session-open.md" 2>/dev/null || true
