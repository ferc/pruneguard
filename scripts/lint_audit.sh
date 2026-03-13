#!/usr/bin/env bash
# lint_audit.sh — Tracks Clippy suppression count to prevent regression.
#
# Usage:
#   ./scripts/lint_audit.sh          # check current count against baseline
#   ./scripts/lint_audit.sh --update # update the baseline to the current count

set -euo pipefail

BASELINE_FILE="$(dirname "$0")/../.clippy-baseline"
SEARCH_ROOT="$(dirname "$0")/.."

# Count #[allow(clippy:: and #![allow(clippy:: in Rust source files,
# excluding node_modules, target, and .release directories.
current=$(grep -r '#\[allow(clippy::' "$SEARCH_ROOT" \
    --include='*.rs' \
    -l \
    | grep -v node_modules \
    | grep -v /target/ \
    | grep -v /.release/ \
    | xargs grep -c '#\[allow(clippy::' \
    | awk -F: '{s+=$2} END {print s}')

# Also count crate-level #![allow(clippy::
crate_level=$(grep -r '#!\[allow(clippy::' "$SEARCH_ROOT" \
    --include='*.rs' \
    -l \
    | grep -v node_modules \
    | grep -v /target/ \
    | grep -v /.release/ \
    | xargs grep -c '#!\[allow(clippy::' \
    | awk -F: '{s+=$2} END {print s}')

total=$((current + crate_level))

if [ "${1:-}" = "--update" ]; then
    echo "$total" > "$BASELINE_FILE"
    echo "Baseline updated to $total suppressions."
    exit 0
fi

if [ ! -f "$BASELINE_FILE" ]; then
    echo "No baseline file found at $BASELINE_FILE"
    echo "Run: ./scripts/lint_audit.sh --update"
    exit 1
fi

baseline=$(cat "$BASELINE_FILE")

echo "Clippy suppressions: $total (baseline: $baseline)"

if [ "$total" -gt "$baseline" ]; then
    echo "FAIL: Suppression count increased from $baseline to $total."
    echo "If the new suppression is justified, run: ./scripts/lint_audit.sh --update"
    exit 1
fi

if [ "$total" -lt "$baseline" ]; then
    echo "Nice — suppression count decreased. Consider updating the baseline:"
    echo "  ./scripts/lint_audit.sh --update"
fi

echo "OK"
