#!/usr/bin/env bash
# Measure pull-request review latency for a repository.
#
# The headline metric is SETTLED WAIT: the time from the author's last push to
# the first human review. That is the wait after the author stopped working,
# which is the only part a nudge could remove. Time-from-PR-open overstates the
# problem badly when authors keep pushing while they wait.
#
# Usage:
#   scripts/pr-latency.sh OWNER/REPO [SINCE_YYYY-MM-DD] [THRESHOLD_HOURS]
#
# Requires: gh (authenticated), jq.
# Cost: roughly one GraphQL point per 50 pull requests.

set -euo pipefail

REPO="${1:?usage: pr-latency.sh OWNER/REPO [SINCE_YYYY-MM-DD] [THRESHOLD_HOURS]}"
SINCE="${2:-$(date -u -v-90d +%Y-%m-%d 2>/dev/null || date -u -d '90 days ago' +%Y-%m-%d)}"
THRESHOLD="${3:-24}"

command -v gh >/dev/null || { echo "gh not found" >&2; exit 4; }
command -v jq >/dev/null || { echo "jq not found" >&2; exit 4; }

RAW=$(mktemp -t pr-latency-XXXXXX.ndjson)
trap 'rm -f "$RAW"' EXIT

echo "Collecting merged PRs in $REPO since $SINCE ..." >&2

cursor=""
page=0
while :; do
  page=$((page + 1))
  after="null"
  [ -n "$cursor" ] && after="\"$cursor\""

  resp=$(gh api graphql -f query="
  query {
    search(query: \"repo:$REPO is:pr is:merged merged:>=$SINCE\", type: ISSUE, first: 50, after: $after) {
      pageInfo { hasNextPage endCursor }
      nodes { ... on PullRequest {
        number createdAt mergedAt additions deletions
        author { login }
        mergedBy { login }
        reviews(first: 20) { nodes { submittedAt state author { login } } }
        timelineItems(last: 20, itemTypes: [PULL_REQUEST_COMMIT]) {
          nodes { ... on PullRequestCommit { commit { committedDate } } } }
      } } }
  }" 2>/dev/null) || resp=""

  if ! printf '%s' "$resp" | jq -e '.data.search' >/dev/null 2>&1; then
    echo "  page $page failed; retrying once" >&2
    sleep 3
    continue
  fi

  printf '%s' "$resp" | jq -c '.data.search.nodes[]' >>"$RAW"
  [ "$(printf '%s' "$resp" | jq -r '.data.search.pageInfo.hasNextPage')" = "true" ] || break
  cursor=$(printf '%s' "$resp" | jq -r '.data.search.pageInfo.endCursor')

  # The GitHub search API caps at 1000 results. Beyond that, narrow SINCE and
  # run again rather than silently reporting a truncated population.
  if [ "$page" -ge 20 ]; then
    echo "  hit the 1000-result search cap; narrow SINCE and re-run" >&2
    break
  fi
done

COUNT=$(wc -l <"$RAW" | tr -d ' ')
[ "$COUNT" -gt 0 ] || { echo "No merged PRs found." >&2; exit 3; }

jq -s -r --argjson threshold "$THRESHOLD" --arg repo "$REPO" --arg since "$SINCE" '
def isbot(l): (l | test("copilot|codex|renovate|dependabot|\\[bot\\]"; "i"));
def pct(a; q): if (a|length) == 0 then null else a[(((a|length)-1)*q)|round] end;
def h(x): if x == null then "—" else (((x*10|round)/10|tostring) + "h") end;

unique_by(.number)
| [ .[]
    | select(isbot(.author.login) | not)
    | select(.mergedAt != null)
    | . as $p
    | ($p.createdAt | fromdateiso8601) as $c
    | ($p.mergedAt  | fromdateiso8601) as $m
    | ([ $p.reviews.nodes[]
         | select(isbot(.author.login) | not)
         | select(.author.login != $p.author.login) ]) as $hr
    | ([ $hr[] | (.submittedAt | fromdateiso8601) ] | if length > 0 then min else null end) as $fhr
    | ([ $p.timelineItems.nodes[] | .commit.committedDate | select(. != null) | fromdateiso8601 ]) as $commits
    | ([ $p.reviews.nodes[]
         | select(.state == "APPROVED")
         | select((.submittedAt | fromdateiso8601) <= $m) ]
       | if length > 0 then (sort_by(.submittedAt) | last) else null end) as $approval
    | { number: $p.number,
        author: $p.author.login,
        churn: ($p.additions + $p.deletions),
        reviewers: [ $hr[].author.login ],
        open_to_review: (if $fhr == null then null else ($fhr - $c)/3600 end),
        settled_wait: (if $fhr == null then null
                       else ([ $commits[] | select(. < $fhr) ]
                             | if length > 0 then ($fhr - max)/3600 else ($fhr - $c)/3600 end)
                       end),
        self_block: (if $approval == null or ($approval.author.login == $p.mergedBy.login) then null
                     else ($m - ($approval.submittedAt | fromdateiso8601))/3600 end) } ]
| . as $all
| ([ $all[] | select(.settled_wait != null) | .settled_wait ] | sort) as $S
| ([ $all[] | select(.open_to_review != null) | .open_to_review ] | sort) as $O
| ([ $all[] | select(.self_block != null) | .self_block ] | sort) as $B
| ([ $S[] | select(. > $threshold) ] | length) as $over
|
"PR review latency — \($repo), merged since \($since)",
"",
"population           \($all|length) PRs   human-reviewed \($S|length)   unreviewed \(($all|length) - ($S|length))",
"",
"SETTLED WAIT   last push -> first human review   (the removable wait)",
"   p50 \(h(pct($S;0.5)))   p75 \(h(pct($S;0.75)))   p90 \(h(pct($S;0.9)))   p95 \(h(pct($S;0.95)))   max \(h(pct($S;1.0)))",
"   over \($threshold)h: \($over) of \($S|length) (\(if ($S|length) > 0 then (($over/($S|length))*100|round) else 0 end)%)  <- a threshold nudge would fire here and stay silent elsewhere",
"",
"OPEN -> FIRST HUMAN REVIEW   (overstates: authors keep pushing while waiting)",
"   p50 \(h(pct($O;0.5)))   p90 \(h(pct($O;0.9)))",
"",
"AUTHOR SELF-BLOCK   approved by someone else, author merged",
"   n=\($B|length)   p50 \(h(pct($B;0.5)))   p90 \(h(pct($B;0.9)))   max \(h(pct($B;1.0)))",
"",
"SETTLED WAIT BY PR SIZE   (if these are flat, the wait is attention, not effort)",
( [ $all[] | select(.settled_wait != null) ]
  | group_by(if .churn < 200 then "<200" elif .churn < 1000 then "200-1k" elif .churn < 5000 then "1k-5k" else ">5k" end)
  | .[] | . as $g | ([ $g[].settled_wait ] | sort) as $x
  | "   \($g[0] | if .churn < 200 then "<200 " elif .churn < 1000 then "200-1k" elif .churn < 5000 then "1k-5k " else ">5k   " end)  n=\($g|length)   p50 \(h(pct($x;0.5)))   p90 \(h(pct($x;0.9)))" ),
"",
"REVIEW LOAD",
( [ $all[].reviewers[] ] | group_by(.) | map({r: .[0], n: length}) | sort_by(-.n) | .[]
  | "   \(.n) reviews   \(.r)" ),
"",
"SLOWEST — candidates a nudge would have caught",
( [ $all[] | select(.settled_wait != null) | select(.settled_wait > $threshold) ]
  | sort_by(-.settled_wait) | .[0:10][]
  | "   #\(.number)  \(h(.settled_wait)) settled  \(.author)" )
' "$RAW"
