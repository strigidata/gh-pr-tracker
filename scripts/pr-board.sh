#!/usr/bin/env bash
# Open-PR board: every open pull request and the reason it cannot merge yet.
#
# Usage:
#   scripts/pr-board.sh OWNER/REPO            # render once
#   watch -n 300 -c scripts/pr-board.sh OWNER/REPO   # live dashboard
#
# Requires: gh (authenticated), jq. Costs ~1 GraphQL point per 50 PRs.

set -euo pipefail

REPO="${1:?usage: pr-board.sh OWNER/REPO}"
OWNER="${REPO%%/*}"
NAME="${REPO##*/}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

command -v gh >/dev/null || { echo "gh not found" >&2; exit 4; }
command -v jq >/dev/null || { echo "jq not found" >&2; exit 4; }

# mergeStateStatus still requires the merge-info preview media type.
RESP=$(gh api graphql \
  -H "Accept: application/vnd.github.merge-info-preview+json" \
  -F owner="$OWNER" -F name="$NAME" \
  -f query='
query($owner:String!, $name:String!) {
  rateLimit { cost remaining }
  viewer { login }
  repository(owner:$owner, name:$name) {
    pullRequests(first: 50, states: OPEN, orderBy:{field: UPDATED_AT, direction: DESC}) {
      totalCount
      nodes {
        number title url isDraft createdAt headRefOid
        author { login }
        reviewDecision
        mergeable
        mergeStateStatus
        reviewRequests(first: 10) {
          nodes { requestedReviewer { __typename ... on User { login } ... on Team { slug } } } }
        latestReviews(first: 10) { nodes { state author { login } submittedAt } }
        reviewThreads(first: 100) { totalCount nodes { isResolved isOutdated } }
        comments(first: 1) { totalCount }
        commits(last: 1) { nodes { commit {
          committedDate
          statusCheckRollup { state contexts(first: 1) {
            checkRunCount checkRunCountsByState { state count }
            statusContextCount statusContextCountsByState { state count } } } } } }
      }
    }
  }
}') || { echo "GraphQL request failed" >&2; exit 5; }

if ! printf '%s' "$RESP" | jq -e '.data.repository' >/dev/null 2>&1; then
  printf '%s' "$RESP" | jq -r '.errors // .' >&2
  exit 5
fi

printf '\033[1m%s\033[0m   \033[2m%s\033[0m\n\n' \
  "$REPO" "$(date '+%H:%M:%S')"

printf '%s' "$RESP" | jq -r -f "$HERE/pr-board.jq"
