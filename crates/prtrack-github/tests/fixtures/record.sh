#!/usr/bin/env bash
# Re-record the fixtures these tests run against, redacted, in one command.
#
#   crates/prtrack-github/tests/fixtures/record.sh OWNER/REPO [PAGE_SIZE]
#
# Records two pages of open pull requests from the live API, replaces the user
# data in them with synthetic values, and writes them here. The pages together
# must be the whole list, so PAGE_SIZE should be about half the repository's
# open pull request count -- the script checks that and refuses to write a pair
# that is not complete.
#
# Requires `gh` (authenticated) and `jq`. Costs about 3 GraphQL points per page.
#
# # Why this is a script and not an edit
#
# A hand-edited fixture drifts from what the API actually returns, and being
# shaped exactly like a real response is the only reason these files are worth
# having. So the redaction is mechanical, deterministic, and re-runnable:
# recording again produces the same synthetic names for the same real ones, and
# the diff of a re-recording shows what changed at GitHub rather than what
# changed in someone's editor.
#
# # What is redacted
#
# Logins, team slugs, pull request titles, branch names, check names, and the
# owner and repository name inside a URL. See redact.jq, which is where the
# rules live and which explains what is deliberately left alone.
#
# Check names are redacted too, and that is the default. They are the join key
# of the whole policy decision -- a required context is matched against a
# reported name literally -- so both sides map through the same table, which
# preserves the join exactly: measured on a real recording, every pull request
# has the same attempt count, distinct-name count, required-check match count
# and duplicate-group count before and after.
#
# They are redacted because a real repository's check names describe its
# internal service and package layout fairly completely. Keeping them is
# possible and deliberate:
#
#   REDACT_CHECK_NAMES=no crates/prtrack-github/tests/fixtures/record.sh OWNER/REPO
#
# The mode is printed on every run, because a variable that does not survive
# whatever invoked this script would otherwise leave real names in a file whose
# commit message says they were redacted.

set -euo pipefail

REPO="${1:?usage: record.sh OWNER/REPO [PAGE_SIZE]}"
PAGE_SIZE="${2:-19}"
REDACT_CHECK_NAMES="${REDACT_CHECK_NAMES:-yes}"

OWNER="${REPO%%/*}"
NAME="${REPO##*/}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
QUERY="$HERE/../../src/open_pull_requests.graphql"

PAGE_1="$HERE/open-pull-requests-page-1.json"
PAGE_2="$HERE/open-pull-requests-page-2.json"

# mergeStateStatus still requires the merge-info preview media type.
PREVIEW="Accept: application/vnd.github.merge-info-preview+json"

command -v gh >/dev/null || { echo "gh not found" >&2; exit 4; }
command -v jq >/dev/null || { echo "jq not found" >&2; exit 4; }
[ -f "$QUERY" ] || { echo "query not found beside the crate: $QUERY" >&2; exit 4; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

fetch() { # fetch CURSOR_OR_EMPTY OUTPUT
  local cursor="$1" out="$2"
  if [ -z "$cursor" ]; then
    gh api graphql -H "$PREVIEW" \
      -F owner="$OWNER" -F name="$NAME" -F first="$PAGE_SIZE" \
      -F query=@"$QUERY" > "$out"
  else
    gh api graphql -H "$PREVIEW" \
      -F owner="$OWNER" -F name="$NAME" -F first="$PAGE_SIZE" -F after="$cursor" \
      -F query=@"$QUERY" > "$out"
  fi
  jq -e '.data.repository.pullRequests.nodes' "$out" >/dev/null \
    || { echo "no pull requests in the response; GitHub said:" >&2
         jq -r '.errors // .' "$out" >&2; exit 5; }
}

# Printed rather than assumed. An environment variable that does not survive
# whatever invoked this script would otherwise leave real check names in a file
# whose commit message says they were redacted, and nothing would say so.
case "$REDACT_CHECK_NAMES" in
  yes) echo "recording $REPO at $PAGE_SIZE per page, redacting check names" >&2 ;;
  no)  echo "recording $REPO at $PAGE_SIZE per page, KEEPING check names" >&2 ;;
  *)   echo "REDACT_CHECK_NAMES must be yes or no, got: $REDACT_CHECK_NAMES" >&2; exit 4 ;;
esac
fetch "" "$work/1.json"
cursor="$(jq -r '.data.repository.pullRequests.pageInfo.endCursor' "$work/1.json")"
fetch "$cursor" "$work/2.json"

# The pair has to be the whole list. A first page that ends the list, or a
# second page that does not, gives fixtures that cannot exercise pagination.
jq -e '.data.repository.pullRequests.pageInfo.hasNextPage == true' "$work/1.json" >/dev/null \
  || { echo "page 1 already ends the list: lower PAGE_SIZE" >&2; exit 6; }
jq -e '.data.repository.pullRequests.pageInfo.hasNextPage == false' "$work/2.json" >/dev/null \
  || { echo "page 2 does not end the list: raise PAGE_SIZE" >&2; exit 6; }

# `hasNextPage: false` is not the same as having everything. The list is ordered
# by UPDATED_AT, so a pull request touched between the two requests moves to the
# front and the cursor steps straight over it -- GitHub still says the list
# ended, and the pair is quietly short. That is the precise failure this crate
# exists to catch, so the recorder is not allowed to produce it: the two pages
# must add up to a total that did not itself move while they were being fetched.
counted="$(jq -s -r '
  (.[0].data.repository.pullRequests.totalCount) as $t1
  | (.[1].data.repository.pullRequests.totalCount) as $t2
  | ((.[0].data.repository.pullRequests.nodes | length)
     + (.[1].data.repository.pullRequests.nodes | length)) as $n
  | "\($t1) \($t2) \($n)"
' "$work/1.json" "$work/2.json")"
read -r total_1 total_2 collected <<< "$counted"
if [ "$total_1" != "$total_2" ] || [ "$collected" != "$total_1" ]; then
  echo "the list moved while it was being recorded:" >&2
  echo "  page 1 said $total_1 open, page 2 said $total_2, and $collected came back" >&2
  echo "  nothing was written. Run it again when the repository is quieter." >&2
  exit 6
fi

jq -s -c \
  --arg owner "$OWNER" --arg name "$NAME" \
  --arg redact_check_names "$REDACT_CHECK_NAMES" \
  -f "$HERE/redact.jq" "$work/1.json" "$work/2.json" > "$work/redacted.json"

jq -c '.[0]' "$work/redacted.json" > "$work/out-1.json"
jq -c '.[1]' "$work/redacted.json" > "$work/out-2.json"

# Post-conditions. A redactor is worth exactly what its checks are worth, so
# these run every time rather than living in someone's memory of one careful
# afternoon.

# 1. Nothing that went in comes out. Every real login, the owner, and the
#    repository name are searched for as plain text across both files.
secrets="$(jq -s -r --arg owner "$OWNER" --arg name "$NAME" '
  ([.[] | .. | objects | select(has("login")) | .login]
   + [.[] | .. | objects | select(has("slug")) | .slug]
   + [.[].data.repository.pullRequests.nodes[]?.title]
   + [.[].data.repository.pullRequests.nodes[]?.baseRefName]
   + [$owner, $name])
  | map(select(type == "string")) | unique | .[]
' "$work/1.json" "$work/2.json")"
while IFS= read -r secret; do
  [ -n "$secret" ] || continue
  case "$secret" in main|master|develop|trunk) continue ;; esac
  if grep -qF -- "$secret" "$work/out-1.json" "$work/out-2.json"; then
    echo "redaction leaked a value: $secret" >&2
    exit 7
  fi
done <<< "$secrets"

# 1b. When check names are being redacted, none of the real ones survives as a
#     field value. Compared whole rather than by substring: a check name is
#     always an entire value, and a base64 cursor can contain a short word by
#     chance. This is the check that would have caught the run where the
#     environment variable did not reach the script and the files came out with
#     every real check name still in them.
if [ "$REDACT_CHECK_NAMES" = "yes" ]; then
  survivors="$(jq -s -r --slurpfile out <(cat "$work/out-1.json" "$work/out-2.json") '
    ([.[].data.repository.pullRequests.nodes[]?.commits.nodes[]?.commit.statusCheckRollup?.contexts.nodes[]?
        | (.name, .context)]
       + [.[].data.repository.branchProtectionRules.nodes[]?.requiredStatusCheckContexts[]?]
       | map(select(type == "string")) | unique) as $names
    | ([$out[] | .. | strings] | unique) as $written
    | [$names[] | select(. as $n | $written | index($n))] | .[]
  ' "$work/1.json" "$work/2.json")"
  if [ -n "$survivors" ]; then
    echo "check-name redaction was asked for but these survived:" >&2
    echo "$survivors" | head -5 >&2
    exit 7
  fi
fi

# 2. The shape is identical: same paths, same array lengths, same types, same
#    null-versus-absent. This is what stops a redactor quietly turning the
#    fixtures into something the API would never send.
shape() {
  jq -S -c '[paths as $p | {p: ($p | map(tostring) | join(".")), t: (getpath($p) | type)}]' "$1"
}
for page in 1 2; do
  if ! diff <(shape "$work/$page.json") <(shape "$work/out-$page.json") >/dev/null; then
    echo "redaction changed the shape of page $page" >&2
    diff <(shape "$work/$page.json") <(shape "$work/out-$page.json") | head -20 >&2
    exit 8
  fi
done

# 3. Redacting twice gives the same bytes, or the mapping is not deterministic
#    and a re-recording would churn the diff for no reason.
jq -s -c --arg owner "$OWNER" --arg name "$NAME" \
  --arg redact_check_names "$REDACT_CHECK_NAMES" \
  -f "$HERE/redact.jq" "$work/1.json" "$work/2.json" | jq -c '.[0]' > "$work/again-1.json"
cmp -s "$work/out-1.json" "$work/again-1.json" \
  || { echo "redaction is not deterministic" >&2; exit 9; }

mv "$work/out-1.json" "$PAGE_1"
mv "$work/out-2.json" "$PAGE_2"

echo "checks passed: nothing leaked, shape unchanged, redaction deterministic" >&2
echo >&2

# A handful of constants in src/wire.rs are pinned to a particular recording,
# on purpose: a count derived from the fixture it is checking would assert
# nothing. Print their new values rather than leaving someone to find out from
# a failing test which ones moved.
jq -s -r '
  "wrote \(.[0].data.repository.pullRequests.nodes | length)"
  + " + \(.[1].data.repository.pullRequests.nodes | length)"
  + " of \(.[0].data.repository.pullRequests.totalCount) open pull requests",
  "",
  "update these in src/wire.rs and its tests:",
  "  TOTAL_OPEN         = \(.[0].data.repository.pullRequests.totalCount)",
  "  PER_PAGE           = \(.[0].data.repository.pullRequests.nodes | length)",
  "  PAGE_1_END_CURSOR  = \"\(.[0].data.repository.pullRequests.pageInfo.endCursor)\"",
  "  first pull request = \(.[0].data.repository.pullRequests.nodes[0].number)",
  "and in src/client.rs tests:",
  "  viewer             = \"\(.[0].data.viewer.login)\""
' "$PAGE_1" "$PAGE_2" >&2
