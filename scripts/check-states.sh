#!/usr/bin/env bash
# Report every CI check state a repository is actually producing, and fail if
# any of them is one the crate's wire-value table does not recognise.
#
# Why this exists. GitHub adds enum values, and an unrecognised one falls to
# `CheckState::Unknown`, which classifies as *running*. A terminal state landing
# there renders as "CI running" forever, because no later attempt is coming.
# That is invisible in the board and invisible in the tests, which is how
# CANCELLED -- the fourth most common state on the target repository -- went
# unnoticed until it was found by hand.
#
# The recognised vocabulary is read out of the Rust source rather than
# duplicated here, so this cannot drift from the thing it checks. It greps
# the match arms, not the function name, so renaming the reader does not
# break it.
#
# Usage:
#   scripts/check-states.sh OWNER/REPO
#
# Exit: 0 all states recognised · 2 unrecognised state found · 4 missing tool
#
# Worth running against any repository prtrack is pointed at, not just this
# one: a different CI setup produces a different state distribution.

set -euo pipefail

REPO="${1:?usage: check-states.sh OWNER/REPO}"
OWNER="${REPO%%/*}"
NAME="${REPO##*/}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SOURCE="$HERE/../crates/prtrack-core/src/checks.rs"

command -v gh >/dev/null || { echo "gh not found" >&2; exit 4; }
command -v jq >/dev/null || { echo "jq not found" >&2; exit 4; }
[ -f "$SOURCE" ] || { echo "cannot find $SOURCE" >&2; exit 4; }

# The parse table is the single source of truth for what is recognised.
KNOWN=$(grep -oE '"[A-Z_]+" *=>' "$SOURCE" | grep -oE '[A-Z_]+' | jq -R . | jq -sc .)
KNOWN_COUNT=$(printf '%s' "$KNOWN" | jq 'length')
[ "$KNOWN_COUNT" -gt 0 ] || { echo "read no states from $SOURCE - has the parse table changed shape?" >&2; exit 4; }

RESP=$(gh api graphql -F owner="$OWNER" -F name="$NAME" -f query='
query($owner:String!, $name:String!) {
  repository(owner:$owner, name:$name) {
    pullRequests(first: 50, states: OPEN) {
      nodes { number
        commits(last: 1) { nodes { commit {
          statusCheckRollup { contexts(first: 100) { nodes {
            __typename
            ... on CheckRun { conclusion status }
            ... on StatusContext { state } } } } } } } }
    }
  }
}') || { echo "GraphQL request failed" >&2; exit 4; }

printf '%s' "$RESP" | jq -r --argjson known "$KNOWN" --arg repo "$REPO" --argjson n "$KNOWN_COUNT" '
[ .data.repository.pullRequests.nodes[]
  | .commits.nodes[0].commit.statusCheckRollup.contexts.nodes[]?
  | if .__typename == "CheckRun" then (.conclusion // .status) else .state end
  | select(. != null) ] as $seen
| ($seen | group_by(.) | map({state: .[0], n: length}) | sort_by(-.n)) as $freq
| ($seen | unique | map(select(. as $s | $known | index($s) | not))) as $unknown
|
"check states on \($repo)  (\($seen|length) checks, vocabulary of \($n) recognised)",
"",
($freq[] | "  \(.n | tostring | (" " * (6 - length)) + .)  \(.state)\(if (.state as $s | $known | index($s)) then "" else "   <-- NOT RECOGNISED" end)"),
"",
(if ($unknown | length) == 0
 then "all states recognised"
 else "UNRECOGNISED: \($unknown | join(", "))\n\nEach of these becomes CheckState::Unknown and classifies as running.\nIf any is a terminal state it will render as \"CI running\" forever.\nAdd it to the parse table in crates/prtrack-core/src/checks.rs and decide\nwhether it is failing or passing." end)
'

UNKNOWN_N=$(printf '%s' "$RESP" | jq --argjson known "$KNOWN" '
[ .data.repository.pullRequests.nodes[]
  | .commits.nodes[0].commit.statusCheckRollup.contexts.nodes[]?
  | if .__typename == "CheckRun" then (.conclusion // .status) else .state end
  | select(. != null) ] | unique | map(select(. as $s | $known | index($s) | not)) | length')

[ "$UNKNOWN_N" -eq 0 ] || exit 2
