#!/usr/bin/env bash

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/helpers.sh"

validator="${SKILL_DIR}/scripts/validate-stack.sh"
setup_linear_repo validate
setup_fake_stack
trunk_sha=$(git -C "${TEST_REPO}" rev-parse main)

(cd "${TEST_REPO}" && bash "${validator}" --trunk "${trunk_sha}" bottom top) >/dev/null
(cd "${TEST_REPO}" && bash "${validator}" --published --trunk "${trunk_sha}" bottom top) >/dev/null

# GitHub does not create a remote stack object for a single pull request. The
# live PR contract is still sufficient to validate a one-layer local stack.
singleton_stack=${FAKE_STACK_JSON}
singleton_remote_stack=${FAKE_REMOTE_STACK_JSON}
export FAKE_STACK_JSON FAKE_REMOTE_STACK_JSON
FAKE_STACK_JSON=$(printf '%s' "${singleton_stack}" | jq -c '.currentBranch = "bottom" | .branches = [.branches[0]]')
FAKE_REMOTE_STACK_JSON='[]'
git -C "${TEST_REPO}" switch --quiet bottom
(cd "${TEST_REPO}" && bash "${validator}" --published --trunk "${trunk_sha}" bottom) >/dev/null

valid_singleton_pr=${FAKE_PR_11_JSON}
export FAKE_PR_11_JSON
FAKE_PR_11_JSON=$(printf '%s' "${valid_singleton_pr}" | jq -c '.state = "CLOSED"')
assert_fails_with "expected OPEN" bash -c 'cd "$1" && bash "$2" --published --trunk "$3" bottom' _ "${TEST_REPO}" "${validator}" "${trunk_sha}"
FAKE_PR_11_JSON=$(printf '%s' "${valid_singleton_pr}" | jq -c '.baseRefName = "develop"')
assert_fails_with "base branch" bash -c 'cd "$1" && bash "$2" --published --trunk "$3" bottom' _ "${TEST_REPO}" "${validator}" "${trunk_sha}"
FAKE_PR_11_JSON=$(printf '%s' "${valid_singleton_pr}" | jq -c '.headRefName = "other"')
assert_fails_with "head branch" bash -c 'cd "$1" && bash "$2" --published --trunk "$3" bottom' _ "${TEST_REPO}" "${validator}" "${trunk_sha}"
FAKE_PR_11_JSON=$(printf '%s' "${valid_singleton_pr}" | jq -c --arg stale "${trunk_sha}" '.headRefOid = $stale')
assert_fails_with "origin/bottom" bash -c 'cd "$1" && bash "$2" --published --trunk "$3" bottom' _ "${TEST_REPO}" "${validator}" "${trunk_sha}"
FAKE_PR_11_JSON=${valid_singleton_pr}

FAKE_REMOTE_STACK_JSON=$(printf '%s' "${singleton_remote_stack}" | jq -c '[.[0] | .pull_requests = [.pull_requests[] | select(.number == 11)]]')
(cd "${TEST_REPO}" && bash "${validator}" --published --trunk "${trunk_sha}" bottom) >/dev/null
FAKE_REMOTE_STACK_JSON=$(printf '%s' "${FAKE_REMOTE_STACK_JSON}" | jq -c '. + .')
assert_fails_with "missing or ambiguous" bash -c 'cd "$1" && bash "$2" --published --trunk "$3" bottom' _ "${TEST_REPO}" "${validator}" "${trunk_sha}"
FAKE_REMOTE_STACK_JSON='{}'
assert_fails_with "must be an array" bash -c 'cd "$1" && bash "$2" --published --trunk "$3" bottom' _ "${TEST_REPO}" "${validator}" "${trunk_sha}"
FAKE_REMOTE_STACK_JSON='[]'

printf 'unpublished singleton change\n' >>"${TEST_REPO}/bottom.txt"
git -C "${TEST_REPO}" add bottom.txt
git -C "${TEST_REPO}" commit --quiet -m "unpublished singleton change"
FAKE_STACK_JSON=$(printf '%s' "${FAKE_STACK_JSON}" | jq -c --arg head "$(git -C "${TEST_REPO}" rev-parse bottom)" '.branches[0].head = $head')
assert_fails_with "not published to origin/bottom" bash -c 'cd "$1" && bash "$2" --published --trunk "$3" bottom' _ "${TEST_REPO}" "${validator}" "${trunk_sha}"
git -C "${TEST_REPO}" reset --quiet --hard 'bottom^'

git -C "${TEST_REPO}" switch --quiet top
FAKE_STACK_JSON=${singleton_stack}
FAKE_REMOTE_STACK_JSON=${singleton_remote_stack}
FAKE_PR_11_JSON=${valid_singleton_pr}

FAKE_REMOTE_STACK_JSON='[]'
assert_fails_with "missing or ambiguous" bash -c 'cd "$1" && bash "$2" --published --trunk "$3" bottom top' _ "${TEST_REPO}" "${validator}" "${trunk_sha}"
FAKE_REMOTE_STACK_JSON=${singleton_remote_stack}

published_stack=${FAKE_STACK_JSON}
printf 'unpublished local change\n' >>"${TEST_REPO}/top.txt"
git -C "${TEST_REPO}" add top.txt
git -C "${TEST_REPO}" commit --quiet -m "unpublished local change"
export FAKE_STACK_JSON
FAKE_STACK_JSON=$(printf '%s' "${published_stack}" | jq -c --arg head "$(git -C "${TEST_REPO}" rev-parse top)" '.branches[1].head = $head')
assert_fails_with "not published to origin/top" bash -c 'cd "$1" && bash "$2" --published --trunk "$3" bottom top' _ "${TEST_REPO}" "${validator}" "${trunk_sha}"
git -C "${TEST_REPO}" reset --quiet --hard 'top^'
export FAKE_STACK_JSON=${published_stack}

refresh_counter="${TEST_ROOT}/stack-view-count"
export FAKE_STACK_VIEW_COUNTER_FILE=${refresh_counter}
export FAKE_STACK_JSON_FIRST
FAKE_STACK_JSON_FIRST=$(printf '%s' "${FAKE_STACK_JSON}" | jq -c --arg stale "${trunk_sha}" '.branches[0].head = $stale')
(cd "${TEST_REPO}" && bash "${validator}" --local --trunk "${trunk_sha}" bottom top) >/dev/null
[[ $(cat "${refresh_counter}") == 2 ]] || fail "expected validation to refresh and then read stack metadata"
unset FAKE_STACK_JSON_FIRST FAKE_STACK_VIEW_COUNTER_FILE

managed_stack=${FAKE_STACK_JSON}
export FAKE_STACK_JSON
FAKE_STACK_JSON=$(printf '%s' "${managed_stack}" | jq -c 'del(.branches[].pr)')
(cd "${TEST_REPO}" && bash "${validator}" --local --trunk "${trunk_sha}" bottom top) >/dev/null
export FAKE_STACK_JSON=${managed_stack}

git -C "${TEST_REPO}" switch --quiet -c unrelated main
printf 'unrelated\n' >"${TEST_REPO}/unrelated.txt"
git -C "${TEST_REPO}" add unrelated.txt
git -C "${TEST_REPO}" commit --quiet -m unrelated
git -C "${TEST_REPO}" switch --quiet top
assert_fails_with "not an ancestor" bash -c 'cd "$1" && bash "$2" --trunk "$3" bottom unrelated' _ "${TEST_REPO}" "${validator}" "${trunk_sha}"

before="${TEST_ROOT}/before.json"
jq -nc \
  --arg trunk "${trunk_sha}" \
  --arg bottom "$(git -C "${TEST_REPO}" rev-parse bottom)" \
  --arg top "$(git -C "${TEST_REPO}" rev-parse top)" \
  '{trunkHead:$trunk,branches:[{name:"bottom",head:$bottom,parentHead:$trunk},{name:"top",head:$top,parentHead:$bottom}]}' >"${before}"
git -C "${TEST_REPO}" switch --quiet bottom
printf 'rewritten parent\n' >>"${TEST_REPO}/bottom.txt"
git -C "${TEST_REPO}" add bottom.txt
git -C "${TEST_REPO}" commit --quiet -m "rewrite parent"
git -C "${TEST_REPO}" switch --quiet top
assert_fails_with "did not move after parent" bash -c 'cd "$1" && bash "$2" --trunk "$3" --before "$4" bottom top' _ "${TEST_REPO}" "${validator}" "${trunk_sha}" "${before}"

setup_merged_history_stack merged-history
trunk_sha=$(git -C "${TEST_REPO}" rev-parse main)
(cd "${TEST_REPO}" && bash "${validator}" --local --trunk "${trunk_sha}" top) >/dev/null
(cd "${TEST_REPO}" && bash "${validator}" --published --trunk "${trunk_sha}" top) >/dev/null

setup_linear_repo conflict-marker-in-json
# Git writes markers at line start; the marker check must win over the JSON check.
printf '{\n<<<<<<< HEAD\n  "version": 1\n=======\n  "version": 2\n>>>>>>> other\n}\n' >"${TEST_REPO}/broken.json"
git -C "${TEST_REPO}" add broken.json
git -C "${TEST_REPO}" commit --quiet -m "add json with conflict markers"
setup_fake_stack
trunk_sha=$(git -C "${TEST_REPO}" rev-parse main)
assert_fails_with "unresolved conflict marker" bash -c 'cd "$1" && bash "$2" --trunk "$3" bottom top' _ "${TEST_REPO}" "${validator}" "${trunk_sha}"

setup_linear_repo malformed-json-artifact
printf '{"version": 1,\n' >"${TEST_REPO}/malformed.json"
git -C "${TEST_REPO}" add malformed.json
git -C "${TEST_REPO}" commit --quiet -m "add malformed json artifact"
setup_fake_stack
trunk_sha=$(git -C "${TEST_REPO}" rev-parse main)
assert_fails_with "invalid JSON" bash -c 'cd "$1" && bash "$2" --trunk "$3" bottom top' _ "${TEST_REPO}" "${validator}" "${trunk_sha}"

setup_linear_repo conflict-marker-in-source
printf 'fn main() {\n<<<<<<< HEAD\n}\n' >"${TEST_REPO}/main.rs"
git -C "${TEST_REPO}" add main.rs
git -C "${TEST_REPO}" commit --quiet -m "add source with conflict marker"
setup_fake_stack
trunk_sha=$(git -C "${TEST_REPO}" rev-parse main)
assert_fails_with "unresolved conflict marker" bash -c 'cd "$1" && bash "$2" --trunk "$3" bottom top' _ "${TEST_REPO}" "${validator}" "${trunk_sha}"

printf 'PASS: stack topology contracts\n'
