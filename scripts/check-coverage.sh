#!/usr/bin/env bash
set -euo pipefail

LCOV_FILE="${1:-lcov.info}"
THRESHOLD="${2:-80}"

if [ ! -f "$LCOV_FILE" ]; then
  echo "❌ $LCOV_FILE not found"
  exit 1
fi

# Parse LCOV: sum LF: and LH:
LINES_FOUND=$(awk '/^LF:/ {sum += $2} END {print sum+0}' "$LCOV_FILE")
LINES_HIT=$(awk '/^LH:/ {sum += $2} END {print sum+0}' "$LCOV_FILE")

if [ "$LINES_FOUND" -eq 0 ]; then
  echo "❌ No lines found in coverage report"
  exit 1
fi

COVERAGE=$(echo "scale=2; ($LINES_HIT / $LINES_FOUND) * 100" | bc)

echo "📊 Coverage: $COVERAGE% (threshold: $THRESHOLD%)"

if (( $(echo "$COVERAGE >= $THRESHOLD" | bc -l) )); then
  echo "✅ Coverage meets threshold"
  exit 0
else
  echo "❌ Coverage $COVERAGE% is below $THRESHOLD%"
  exit 1
fi
