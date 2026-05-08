#/usr/bin/env bash
set -euo pipefail

# Quality enforcement script - used by pre-commit and CI

echo "[enforce-patterns] Checking code quality rules in src/ and tests/..."

VIOLATIONS=0

# Rule 1 & 2: #[ignore] and #[allow] must have // Reason:
if grep -r --include='*.rs' -n '#\[ignore\]' src/ tests/ 2>/dev/null | grep -v -E '// Reason:'; then
  echo "❌ Found #[ignore] without '// Reason:' comment"
  VIOLATIONS=$((VIOLATIONS + 1))
fi

if grep -r --include='*.rs' -n '#\[allow' src/ tests/ 2>/dev/null | grep -v -E '// Reason:'; then
  echo "❌ Found #[allow(...)] without '// Reason:' comment"
  VIOLATIONS=$((VIOLATIONS + 1))
fi

# Rule 3: todo!() and unimplemented!() forbidden in src/ (except tests)
if grep -r --include='*.rs' -n -E 'todo!\(\)|unimplemented!\(\)' src/ 2>/dev/null; then
  echo "❌ Found todo!() or unimplemented!() in src/ (not allowed outside tests)"
  VIOLATIONS=$((VIOLATIONS + 1))
fi

# More rules can be added here

if [ $VIOLATIONS -gt 0 ]; then
  echo "❌ $VIOLATIONS quality violation(s) found"
  exit 1
else
  echo "✅ No quality violations found"
fi
