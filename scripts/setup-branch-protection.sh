#!/usr/bin/env bash
set -euo pipefail

echo "🔧 Setting up branch protection on main..."

if ! gh auth status &> /dev/null; then
  echo "❌ gh CLI not authenticated. Run 'gh auth login' first."
  exit 1
fi

OWNER_REPO=$(gh repo view --json nameWithOwner -q .nameWithOwner)
OWNER=$(echo "$OWNER_REPO" | cut -d/ -f1)
REPO=$(echo "$OWNER_REPO" | cut -d/ -f2)

echo "📌 Repo: $OWNER/$REPO"

gh api -X PUT repos/$OWNER/$REPO/branches/main/protection \
  -f required_status_checks.strict=true \
  -f "required_status_checks.contexts[]=format" \
  -f "required_status_checks.contexts[]=lint" \
  -f "required_status_checks.contexts[]=check" \
  -f "required_status_checks.contexts[]=test" \
  -f "required_status_checks.contexts[]=build" \
  -f "required_status_checks.contexts[]=enforce-patterns" \
  -f "required_status_checks.contexts[]=coverage" \
  -f enforce_admins=true \
  -f required_pull_request_reviews=null \
  -f restrictions=null \
  -f allow_force_pushes=false \
  -f allow_deletions=false \
  -f required_conversation_resolution=true

gh api -X PATCH repos/$OWNER/$REPO -f delete_branch_on_merge=true

echo "✅ Branch protection applied!"
