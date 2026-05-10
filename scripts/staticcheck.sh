#!/usr/bin/env bash
# staticcheck.sh — wrapper to run honnef.co/go/tools/cmd/staticcheck across
# the project. Auto-installs to $(go env GOPATH)/bin/staticcheck on first run.
#
# Usage:
#   ./scripts/staticcheck.sh           # check ./...
#   ./scripts/staticcheck.sh ./internal/hub/   # check a subset
#
# Exits 0 when clean, non-zero on any finding. Pre-U-pre baseline: 7
# findings (4 unused funcs + 2 dead test variables + 1 ignored-return-value
# bug); cleared at U-pre commit (137ea22's follow-up).

set -e

GOPATH=$(go env GOPATH)
STATICCHECK="${GOPATH}/bin/staticcheck"

if [ ! -x "$STATICCHECK" ]; then
  echo "Installing staticcheck to $STATICCHECK..." >&2
  go install honnef.co/go/tools/cmd/staticcheck@latest
fi

if [ $# -eq 0 ]; then
  exec "$STATICCHECK" ./...
else
  exec "$STATICCHECK" "$@"
fi
