#!/usr/bin/env bash

set -euo pipefail

TEST_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
SKILL_DIR=$(cd "${TEST_DIR}/.." && pwd)
REPO_ROOT=$(cd "${TEST_DIR}/../../../.." && pwd)
TEST_ROOT=$(mktemp -d)

# Remove only the disposable repositories created by the current test process.
cleanup_test_root() {
  rm -rf "${TEST_ROOT}"
}

trap cleanup_test_root EXIT

# Fail the current test with a concise message.
fail() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

# Assert that a string contains an expected fragment.
assert_contains() {
  local actual=$1
  local expected=$2

  [[ "${actual}" == *"${expected}"* ]] || fail "expected output to contain '${expected}', got '${actual}'"
}

# Initialize a linear main, bottom, and top repository with a bare origin.
setup_linear_repo() {
  local name=$1

  TEST_REPO="${TEST_ROOT}/${name}"
  TEST_REMOTE="${TEST_ROOT}/${name}-origin.git"
  git init --bare --quiet "${TEST_REMOTE}"
  git init --quiet --initial-branch=main "${TEST_REPO}"
  git -C "${TEST_REPO}" config user.email stacked-pr-tests@example.com
  git -C "${TEST_REPO}" config user.name "Stacked PR Tests"
  printf 'main\n' >"${TEST_REPO}/main.txt"
  git -C "${TEST_REPO}" add main.txt
  git -C "${TEST_REPO}" commit --quiet -m main
  git -C "${TEST_REPO}" remote add origin "${TEST_REMOTE}"
  git -C "${TEST_REPO}" push --quiet --set-upstream origin main
  git -C "${TEST_REPO}" config url."${TEST_REMOTE}".insteadOf https://github.com/acme/gh-pr-tracker.git
  git -C "${TEST_REPO}" remote set-url origin https://github.com/acme/gh-pr-tracker.git
  git --git-dir="${TEST_REMOTE}" symbolic-ref HEAD refs/heads/main
  git -C "${TEST_REPO}" switch --quiet -c bottom
  printf 'bottom\n' >"${TEST_REPO}/bottom.txt"
  git -C "${TEST_REPO}" add bottom.txt
  git -C "${TEST_REPO}" commit --quiet -m bottom
  git -C "${TEST_REPO}" push --quiet --set-upstream origin bottom
  git -C "${TEST_REPO}" switch --quiet -c top
  printf 'top\n' >"${TEST_REPO}/top.txt"
  git -C "${TEST_REPO}" add top.txt
  git -C "${TEST_REPO}" commit --quiet -m top
  git -C "${TEST_REPO}" push --quiet --set-upstream origin top
}

# Export consistent fake GitHub responses for the current linear repository.
setup_fake_stack() {
  local main_sha bottom_sha top_sha

  main_sha=$(git -C "${TEST_REPO}" rev-parse main)
  bottom_sha=$(git -C "${TEST_REPO}" rev-parse bottom)
  top_sha=$(git -C "${TEST_REPO}" rev-parse top)
  export FAKE_GH_VERSION=2.97.0
  export FAKE_STACK_VERSION=0.1.0
  export FAKE_CLI_LATEST=v2.97.0
  export FAKE_STACK_LATEST=v0.1.0
  export FAKE_AUTH_OK=1
  export FAKE_REPOSITORY=acme/gh-pr-tracker
  export FAKE_STACK_JSON
  FAKE_STACK_JSON=$(jq -nc \
    --arg main "${main_sha}" \
    --arg bottom "${bottom_sha}" \
    --arg top "${top_sha}" \
    '{trunk:"main",currentBranch:"top",branches:[{name:"bottom",head:$bottom,base:$main,isCurrent:false,isMerged:false,isQueued:false,needsRebase:false,pr:{number:11,url:"https://github.com/acme/gh-pr-tracker/pull/11",state:"OPEN"}},{name:"top",head:$top,base:$bottom,isCurrent:true,isMerged:false,isQueued:false,needsRebase:false,pr:{number:12,url:"https://github.com/acme/gh-pr-tracker/pull/12",state:"OPEN"}}]}')
  export FAKE_PR_11_JSON
  FAKE_PR_11_JSON=$(jq -nc --arg head "${bottom_sha}" '{number:11,state:"OPEN",baseRefName:"main",headRefName:"bottom",headRefOid:$head}')
  export FAKE_PR_12_JSON
  FAKE_PR_12_JSON=$(jq -nc --arg head "${top_sha}" '{number:12,state:"OPEN",baseRefName:"bottom",headRefName:"top",headRefOid:$head}')
  export FAKE_REMOTE_STACK_JSON
  FAKE_REMOTE_STACK_JSON=$(jq -nc \
    --arg bottom "${bottom_sha}" \
    --arg top "${top_sha}" \
    '[{number:7,base:{ref:"main"},pull_requests:[{number:10,state:"closed",head:{ref:"merged-layer",sha:$bottom}},{number:11,state:"open",head:{ref:"bottom",sha:$bottom}},{number:12,state:"open",head:{ref:"top",sha:$top}}]}]')
  export GH_BIN="${TEST_DIR}/fake-gh.sh"
}

# Set up a stack whose bottom pull request is merged and retained as history.
setup_merged_history_stack() {
  local name=$1 trunk_sha bottom_sha top_sha

  setup_linear_repo "${name}"
  setup_fake_stack
  git -C "${TEST_REPO}" branch --force main bottom
  git -C "${TEST_REPO}" push --quiet --force origin main
  trunk_sha=$(git -C "${TEST_REPO}" rev-parse main)
  bottom_sha=$(git -C "${TEST_REPO}" rev-parse bottom)
  top_sha=$(git -C "${TEST_REPO}" rev-parse top)
  export FAKE_STACK_JSON
  FAKE_STACK_JSON=$(jq -nc \
    --arg main "${trunk_sha}" \
    --arg bottom "${bottom_sha}" \
    --arg top "${top_sha}" \
    '{trunk:"main",currentBranch:"top",branches:[{name:"bottom",head:$bottom,base:$main,isCurrent:false,isMerged:true,isQueued:false,needsRebase:false,pr:{number:11,url:"https://github.com/acme/gh-pr-tracker/pull/11",state:"MERGED"}},{name:"top",head:$top,base:$main,isCurrent:true,isMerged:false,isQueued:false,needsRebase:false,pr:{number:12,url:"https://github.com/acme/gh-pr-tracker/pull/12",state:"OPEN"}}]}')
  export FAKE_PR_12_JSON
  FAKE_PR_12_JSON=$(jq -nc --arg head "${top_sha}" '{number:12,state:"OPEN",baseRefName:"main",headRefName:"top",headRefOid:$head}')
  export FAKE_REMOTE_STACK_JSON
  FAKE_REMOTE_STACK_JSON=$(jq -nc \
    --arg bottom "${bottom_sha}" \
    --arg top "${top_sha}" \
    '[{number:7,base:{ref:"main"},pull_requests:[{number:11,state:"closed",head:{ref:"bottom",sha:$bottom}},{number:12,state:"open",head:{ref:"top",sha:$top}}]}]')
}

# Run a command and require it to fail with the expected fragment.
assert_fails_with() {
  local expected=$1
  shift
  local output status

  set +e
  output=$("$@" 2>&1)
  status=$?
  set -e
  [[ ${status} -ne 0 ]] || fail "expected command to fail: $*"
  assert_contains "${output}" "${expected}"
}
