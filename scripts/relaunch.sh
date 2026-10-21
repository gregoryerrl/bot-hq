#!/usr/bin/env bash
# Relaunch bot-hq from INSIDE a session and come back to it.
#
#   scripts/relaunch.sh [--from <session id>] [--delay <secs>] [--dry-run]
#
# For an agent that needs the app on a new build while the user is away. Two
# things make this more than `./start --release`:
#
#   1. bot-hq SIGKILLs every agent's process GROUP when it exits, so a plain
#      child of the agent's shell dies with the app before it can relaunch
#      anything. This script re-executes itself in a NEW session (setsid)
#      with its stdio on a log file, and the foreground call returns at once.
#   2. After a boot nothing deals a turn to anyone. The caller declares a
#      TEMPORARY halt (`halt(reason, wake_after_secs)`) before its turn ends:
#      the app respawns the sessions holding a wake instant at boot, the halt
#      re-arms, and an expired instant deals the declarer its next turn.
#
# The procedure, from the agent's side:
#      scripts/relaunch.sh --from <own session id> --delay 20
#      halt("Relaunching bot-hq onto <sha> …", wake_after_secs: 240)
#      (end the turn — the stop lands ~20 s later; the build and launch follow)
#
# Refuses when another open session holds a turn (a relaunch kills it
# mid-crunch) — the caller's own turn is the one being cut, by design. Unix
# only: `start` is bash and the group escape is POSIX.

set -euo pipefail

cd "$(dirname "$0")/.."
REPO=$(pwd)
DATA_DIR="${BOT_HQ_DATA_DIR:-$HOME/.bot-hq}"
DB="$DATA_DIR/.local/bot-hq.db"
LOG="$DATA_DIR/.local/relaunch.log"
FROM=""
DELAY=15
DRY_RUN=0
DETACHED=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --from)     FROM="${2:-}"; shift 2 ;;
    --delay)    DELAY="${2:-}"; shift 2 ;;
    --dry-run)  DRY_RUN=1; shift ;;
    --detached) DETACHED=1; shift ;;
    -h|--help)  sed -n '2,27p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1 (try --help)" >&2; exit 2 ;;
  esac
done

[[ "$DELAY" =~ ^[0-9]+$ ]] || { echo "--delay wants a number of seconds" >&2; exit 2; }

stamp() { date -u +%Y-%m-%dT%H:%M:%SZ; }
log() { printf '%s relaunch: %s\n' "$(stamp)" "$1"; }

# Open sessions holding a turn right now, other than the caller's. The app
# writes `current_turn_participant_id` while a turn is dealt and clears it at
# the boundary; a read-only open never blocks the app's writer.
busy_sessions() {
  [[ -f "$DB" ]] || return 0
  command -v sqlite3 >/dev/null || { echo "(sqlite3 missing — busy check skipped)"; return 0; }
  sqlite3 -readonly "$DB" \
    "SELECT id FROM sessions WHERE closed_at IS NULL AND current_turn_participant_id IS NOT NULL AND id != '${FROM//\'/''}';" \
    2>/dev/null || true
}

preflight() {
  [[ -x start ]] || { log "refused: $REPO/start is missing or not executable"; return 1; }
  command -v cargo >/dev/null || { log "refused: cargo is not on PATH ($PATH)"; return 1; }
  command -v npm >/dev/null || { log "refused: npm is not on PATH ($PATH)"; return 1; }
  local busy
  busy=$(busy_sessions)
  if [[ -n "$busy" ]]; then
    log "refused: another session holds a turn — $(echo "$busy" | tr '\n' ' ')(a relaunch would kill it mid-turn)"
    return 1
  fi
  return 0
}

if [[ "$DRY_RUN" -eq 1 ]]; then
  log "dry run from ${FROM:-<no session>} with a ${DELAY}s delay; data dir $DATA_DIR"
  preflight || exit 1
  log "preflight ok — a real run would stop, rebuild and launch the app"
  exit 0
fi

if [[ "$DETACHED" -eq 0 ]]; then
  mkdir -p "$(dirname "$LOG")"
  command -v python3 >/dev/null || { echo "python3 is needed to leave the agent's process group" >&2; exit 2; }
  # A new session for the real run: the app's exit kills the caller's whole
  # process group, and this one must outlive it. stdio on the log, so the
  # caller's tool call is not held open by an inherited pipe.
  python3 -c 'import os, sys; os.setsid(); os.execvp(sys.argv[1], sys.argv[1:])' \
    "$0" --detached --from "$FROM" --delay "$DELAY" </dev/null >>"$LOG" 2>&1 &
  echo "relaunch scheduled: the app stops in ~${DELAY}s, rebuilds and comes back (log: $LOG)"
  exit 0
fi

# ---- the detached run -------------------------------------------------------
log "requested from ${FROM:-<no session>}; stopping in ${DELAY}s (pid $$)"
preflight || exit 1
sleep "$DELAY"
# Re-check at the moment of truth: a turn may have started during the delay.
busy=$(busy_sessions)
if [[ -n "$busy" ]]; then
  log "refused at the stop: another session holds a turn — $(echo "$busy" | tr '\n' ' ')"
  exit 1
fi
log "running ./start --release (stop → frontend → release build → launch)"
# `start` stays as the app's parent and forwards a termination; this process
# is that parent now, detached, like a terminal that nobody closes.
exec ./start --release
