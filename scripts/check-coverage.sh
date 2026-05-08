#!/usr/bin/env bash
set -euo pipefail

LCOV_FILE="${1:-lcov.info}"
THRESHOLD="${2:-80}"

if [ ! -f "$LCOV_FILE" ]; then
  echo "❌ $LCOV_FILE not found"
  exit 1
fi

# Parse LCOV: sum LF: and LH: (LCOV uses ":" as field separator)
LINES_FOUND=$(awk -F: '/^LF:/ {sum += $2} END {print sum+0}' "$LCOV_FILE")
LINES_HIT=$(awk -F: '/^LH:/ {sum += $2} END {print sum+0}' "$LCOV_FILE")

if [ "$LINES_FOUND" -eq 0 ]; then
  echo "❌ No lines found in coverage report"
  exit 1
fi

COVERAGE=$(awk -v hit="$LINES_HIT" -v found="$LINES_FOUND" 'BEGIN { printf "%.2f", (hit / found) * 100 }')

echo "📊 Coverage: $COVERAGE% (threshold: $THRESHOLD%)"

PASSED=$(awk -v cov="$COVERAGE" -v th="$THRESHOLD" 'BEGIN { print (cov + 0 >= th + 0) ? "1" : "0" }')

if [ "$PASSED" = "1" ]; then
  echo "✅ Coverage meets threshold"
  exit 0
else
  echo "❌ Coverage $COVERAGE% is below $THRESHOLD%"
  exit 1
fi
