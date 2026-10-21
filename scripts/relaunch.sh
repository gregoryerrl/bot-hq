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
# mid-crunch) — the caller's own turn is the one being cut, by design —, when
# that cannot be known (no answer from the database is a no), and when the
# caller's temporary halt is gone at the moment of the stop (a user message
# releases it; the boot would then bring nobody back). Unix only: `start` is
# bash and the group escape is POSIX. `tests/relaunch_script_test.rs` runs the
# decision path against a stand-in launcher.

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
    -h|--help)  sed -n '2,29p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown argument: $1 (try --help)" >&2; exit 2 ;;
  esac
done

[[ "$DELAY" =~ ^[0-9]+$ ]] || { echo "--delay wants a number of seconds" >&2; exit 2; }

stamp() { date -u +%Y-%m-%dT%H:%M:%SZ; }
log() { printf '%s relaunch: %s\n' "$(stamp)" "$1"; }

# The launcher. Overridable so a test can run the whole decision path against
# a stand-in that only records that it was reached.
START_CMD="${BOT_HQ_RELAUNCH_START:-./start}"

# One read-only query against the app's database. Prints the rows; FAILS
# (non-zero, the reason on stdout) when the answer cannot be had — no
# database, no sqlite3, a locked or broken file. Every caller treats a
# failure as a refusal: a relaunch kills every live agent, so "unknown" is
# never "nobody is working" (EYES ac3448b8).
db_query() {
  [[ -f "$DB" ]] || { echo "no database at $DB"; return 1; }
  command -v sqlite3 >/dev/null || { echo "sqlite3 is not on PATH"; return 1; }
  local out
  if ! out=$(sqlite3 -readonly "$DB" "$1" 2>&1); then
    echo "sqlite3 failed: $out"
    return 1
  fi
  printf '%s' "$out"
}

# Open sessions holding a turn right now, other than the caller's. The app
# writes `current_turn_participant_id` while a turn is dealt and clears it at
# the boundary; a read-only open never blocks the app's writer.
busy_sessions() {
  db_query "SELECT id FROM sessions WHERE closed_at IS NULL AND current_turn_participant_id IS NOT NULL AND id != '${FROM//\'/''}';"
}

# Nobody else is mid-turn, and the answer is certain. Logs the refusal.
nobody_busy() {
  local busy
  if ! busy=$(busy_sessions); then
    log "refused: the busy-session check could not be made ($busy) — a relaunch kills every live agent, so an unknown answer is a no"
    return 1
  fi
  if [[ -n "$busy" ]]; then
    log "refused: another session holds a turn — $(echo "$busy" | tr '\n' ' ')(a relaunch would kill it mid-turn)"
    return 1
  fi
  return 0
}

# The caller comes back only through its TEMPORARY halt: the boot respawns
# the sessions holding a wake instant, and a user message that lands in the
# meantime releases the halt (it happened on the first run, 2026-10-21). With
# the slot empty the stop would strand the caller, so it is checked at the
# moment of truth (EYES 731ce8e3).
caller_will_wake() {
  [[ -n "$FROM" ]] || return 0
  local wake
  if ! wake=$(db_query "SELECT halt_wake_at FROM sessions WHERE id = '${FROM//\'/''}' AND closed_at IS NULL AND halt_wake_at IS NOT NULL;"); then
    log "refused: the caller's wake instant could not be read ($wake)"
    return 1
  fi
  if [[ -z "$wake" ]]; then
    log "refused: $FROM holds no temporary halt (released by a message, or not declared yet) — nothing would bring it back; declare halt(…, wake_after_secs) and run this again"
    return 1
  fi
  log "$FROM wakes at $wake"
  return 0
}

preflight() {
  [[ -x "$START_CMD" ]] || { log "refused: $START_CMD is missing or not executable"; return 1; }
  command -v cargo >/dev/null || { log "refused: cargo is not on PATH ($PATH)"; return 1; }
  command -v npm >/dev/null || { log "refused: npm is not on PATH ($PATH)"; return 1; }
  nobody_busy
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
# The moment of truth: a turn may have started during the delay, and the
# caller's halt may have been released by a message. Both are re-read now.
nobody_busy || exit 1
caller_will_wake || exit 1
log "running $START_CMD --release (stop → frontend → release build → launch)"
# `start` stays as the app's parent and forwards a termination; this process
# is that parent now, detached, like a terminal that nobody closes. Its
# stdout is dropped: `start` tees the app's output into bot-hq.log already,
# and the first run duplicated the whole app log here. stderr — a failed
# build's errors — stays.
if "$START_CMD" --release 2>&1 >/dev/null; then
  log "the app exited"
  exit 0
fi
# A failed build must not leave the app down while nobody is watching: the
# binary that ran before is still there (a failed compile does not remove it)
# — launch it the way `start` does, and say so where the caller will look.
BIN=target/release/bot-hq
if pgrep -f 'target/(debug|release)/bot-hq$' >/dev/null; then
  log "start failed after the app came up; leaving it running"
  exit 1
fi
[[ -x "$BIN" ]] || { log "start failed and $BIN is missing — the app is DOWN"; exit 1; }
log "start failed (build or launch, see above); launching the previous $BIN so the app is not left down"
rm -f "$DATA_DIR/.local/lock"
"$BIN" 2>&1 | tee -a "$DATA_DIR/.local/bot-hq.log"
log "the previous binary exited"
exit 1
