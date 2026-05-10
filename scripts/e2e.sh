#!/usr/bin/env bash
# Sympheo end-to-end harness driver.
#   ./scripts/e2e.sh                       # mode=mock (default)
#   ./scripts/e2e.sh --mode=claude         # real claude CLI (requires ANTHROPIC_API_KEY)
#   ./scripts/e2e.sh --keep-on-failure     # don't delete the throwaway repo on failure
#   ./scripts/e2e.sh --suite=tests/e2e/suites/01_happy_path.robot
set -euo pipefail

MODE=mock
KEEP_ON_FAILURE=0
SUITE=tests/e2e/suites/

for arg in "$@"; do
  case "$arg" in
    --mode=*)            MODE="${arg#*=}" ;;
    --keep-on-failure)   KEEP_ON_FAILURE=1 ;;
    --suite=*)           SUITE="${arg#*=}" ;;
    -h|--help)
      sed -n '2,7p' "$0"; exit 0 ;;
    *)
      echo "unknown arg: $arg" >&2; exit 2 ;;
  esac
done

# Run from repo root regardless of where the user invoked us.
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
cd "$ROOT"

# Sanity: GITHUB_TOKEN must be set (used by sympheo + by gh).
if [ -z "${GITHUB_TOKEN:-}" ]; then
  echo "GITHUB_TOKEN not set" >&2
  exit 2
fi
if [ "$MODE" = "claude" ] && [ -z "${ANTHROPIC_API_KEY:-}" ]; then
  echo "MODE=claude requires ANTHROPIC_API_KEY" >&2
  exit 2
fi

# 1. Ensure venv with pinned deps.
VENV="$ROOT/tests/e2e/.venv"
if [ ! -d "$VENV" ]; then
  echo "[e2e] creating venv at $VENV"
  python3 -m venv "$VENV"
fi
# shellcheck disable=SC1091
. "$VENV/bin/activate"
pip install -q --upgrade pip
pip install -q -r "$ROOT/tests/e2e/requirements.txt"

# 2. Build sympheo (release).
echo "[e2e] building sympheo (release)"
cargo build --release --quiet

# 3. Run robot.
TS=$(date -u +%Y%m%dT%H%M%SZ)
OUTDIR="$ROOT/tests/e2e/reports/$TS"
mkdir -p "$OUTDIR"
echo "[e2e] mode=$MODE outdir=$OUTDIR"

set +e
robot \
  --outputdir "$OUTDIR" \
  --variable "MODE:$MODE" \
  --variable "KEEP_ON_FAILURE:$KEEP_ON_FAILURE" \
  --variable "SYMPHEO_BIN:$ROOT/target/release/sympheo" \
  "$SUITE"
RC=$?
set -e

echo
echo "[e2e] exit=$RC report=$OUTDIR/report.html"
exit $RC
