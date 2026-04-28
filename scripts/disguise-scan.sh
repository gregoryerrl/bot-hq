#!/usr/bin/env bash
# disguise-scan.sh — pre-commit grep scan for bot-hq leaks in client-repo planning artifacts.
#
# Phase I W1b G-4 ratchet — ships from bot-hq, consumed by client repos
# (bcc-ad-manager, ad-exporter, etc.) and by bot-hq agents themselves
# when working in client-repo planning artifacts. bot-hq-internal repo
# files are EXEMPT (see section 5 of feedback_disguise_scaffold_scan.md
# in user's auto-memory) — agent-name/cycle-id refs are appropriate in
# bot-hq's own docs/arcs/* and similar internal docs.
#
# Usage:
#   disguise-scan.sh <file-or-dir> [<file-or-dir>...]
#   disguise-scan.sh --staged          # scan files git-staged for next commit
#   disguise-scan.sh --help
#
# Exit codes:
#   0 — disguise-clean (no matches found)
#   1 — leak detected (one or more matches)
#   2 — usage error
#
# Pattern source: feedback_disguise_scaffold_scan.md (user auto-memory).
# Refined 2026-04-27 + adjusted Phase I 2026-04-28 W1b commit.

set -euo pipefail

usage() {
    cat <<'EOF'
disguise-scan.sh — scan files/dirs for bot-hq disguise-rule leaks.

USAGE:
    disguise-scan.sh <file-or-dir> [<file-or-dir>...]
    disguise-scan.sh --staged
    disguise-scan.sh --help

DESCRIPTION:
    Greps for agent-name leaks (Brian/Rain/Emma), bot-hq jargon (BRAIN/HANDS/
    EYES/DISC v2/greenflag/trio), hub-message-id refs (msg #NNNN), and
    related disguise-rule violations in target paths.

    Use BEFORE committing client-repo planning artifacts (e.g.,
    bcc-ad-manager docs/plans/*.md). bot-hq-internal docs are exempt.

OUTPUT:
    On match: prints offending lines with file:line context, exits 1.
    On clean: prints "disguise-scan: clean" to stderr, exits 0.

EXAMPLES:
    # Scan a single file
    disguise-scan.sh docs/plans/audit-2026-04-28.md

    # Scan all staged files for the next commit
    disguise-scan.sh --staged

    # Scan a whole directory tree
    disguise-scan.sh docs/

EOF
}

# Canonical regex — verbatim from feedback_disguise_scaffold_scan.md line 13,
# with `\bDISC\s+v2\b` whole-token form to avoid Discord/Discovered FPs.
PATTERN='bot-hq|bot\.hq|\brain\b|\bbrian\b|\bemma\b|\bclive\b|hub_|hub-|/rain|rain/|pre-rain|post-rain|msg #?[0-9]{4}|message #?[0-9]{4}|\beyes\b|\bhands\b|\bbrain\b|greenflag|trio|coder.dispatch|agent ID|[a-f0-9]{8}-mcp|\bDISC\s+v2\b'

if [[ $# -eq 0 ]]; then
    usage >&2
    exit 2
fi

if [[ "$1" == "--help" || "$1" == "-h" ]]; then
    usage
    exit 0
fi

# Resolve --staged into a list of staged files.
if [[ "$1" == "--staged" ]]; then
    if ! git rev-parse --git-dir >/dev/null 2>&1; then
        echo "disguise-scan: --staged requires a git repository" >&2
        exit 2
    fi
    mapfile -t targets < <(git diff --cached --name-only --diff-filter=ACMR)
    if [[ ${#targets[@]} -eq 0 ]]; then
        echo "disguise-scan: no staged files" >&2
        exit 0
    fi
else
    targets=("$@")
fi

# Validate each target exists.
for t in "${targets[@]}"; do
    if [[ ! -e "$t" ]]; then
        echo "disguise-scan: path not found: $t" >&2
        exit 2
    fi
done

# Run the scan. -r recursive (no-op for files), -n line-numbers, -E extended,
# -i case-insensitive (catches BRAIN/Brain/brain). Capture exit so we control
# the report shape.
set +e
output=$(grep -rnEi "$PATTERN" "${targets[@]}" 2>/dev/null)
status=$?
set -e

if [[ $status -eq 0 ]]; then
    # grep found matches — print them, exit 1.
    echo "disguise-scan: leak detected" >&2
    echo "$output"
    exit 1
elif [[ $status -eq 1 ]]; then
    # grep found nothing — clean.
    echo "disguise-scan: clean" >&2
    exit 0
else
    # grep had a real error.
    echo "disguise-scan: grep error (status $status)" >&2
    exit 2
fi
