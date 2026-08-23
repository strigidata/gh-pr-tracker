#!/usr/bin/env bash

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/helpers.sh"

preflight="${SKILL_DIR}/scripts/preflight.sh"
setup_linear_repo preflight
setup_fake_stack

(cd "${TEST_REPO}" && bash "${preflight}" managed) >/dev/null
(cd "${TEST_REPO}" && bash "${preflight}" adopt bottom top) >/dev/null

valid_repository=${FAKE_REPOSITORY}
export FAKE_REPOSITORY=acme/gh-pr-tracker-alt
assert_fails_with "origin repository" bash -c 'cd "$1" && bash "$2" managed' _ "${TEST_REPO}" "${preflight}"
export FAKE_REPOSITORY=${valid_repository}

git -C "${TEST_REPO}" switch --quiet -c parallel main
printf 'parallel\n' >"${TEST_REPO}/parallel.txt"
git -C "${TEST_REPO}" add parallel.txt
git -C "${TEST_REPO}" commit --quiet -m parallel
(cd "${TEST_REPO}" && bash "${preflight}" adopt bottom parallel) >/dev/null
git -C "${TEST_REPO}" switch --quiet top

export FAKE_STACK_VERSION=0.0.9
assert_fails_with "gh-stack 0.0.9 is stale" bash -c 'cd "$1" && bash "$2" managed' _ "${TEST_REPO}" "${preflight}"
export FAKE_STACK_VERSION=0.1.0

export FAKE_CLI_LOOKUP_OK=0
assert_fails_with "latest stable gh version" bash -c 'cd "$1" && bash "$2" managed' _ "${TEST_REPO}" "${preflight}"
export FAKE_CLI_LOOKUP_OK=1

export FAKE_AUTH_OK=0
assert_fails_with "GitHub CLI authentication" bash -c 'cd "$1" && bash "$2" managed' _ "${TEST_REPO}" "${preflight}"
export FAKE_AUTH_OK=1

printf 'dirty\n' >>"${TEST_REPO}/top.txt"
assert_fails_with "working tree is not clean" bash -c 'cd "$1" && bash "$2" managed' _ "${TEST_REPO}" "${preflight}"
git -C "${TEST_REPO}" restore top.txt

valid_pr_12=${FAKE_PR_12_JSON}
export FAKE_PR_12_JSON
FAKE_PR_12_JSON=$(printf '%s' "${valid_pr_12}" | jq -c '.baseRefName = "main"')
assert_fails_with "base branch" bash -c 'cd "$1" && bash "$2" managed' _ "${TEST_REPO}" "${preflight}"
export FAKE_PR_12_JSON=${valid_pr_12}

FAKE_PR_12_JSON=$(printf '%s' "${valid_pr_12}" | jq -c '.state = "CLOSED"')
assert_fails_with "expected OPEN" bash -c 'cd "$1" && bash "$2" managed' _ "${TEST_REPO}" "${preflight}"
export FAKE_PR_12_JSON=${valid_pr_12}

FAKE_PR_12_JSON=$(printf '%s' "${valid_pr_12}" | jq -c --arg stale "$(git -C "${TEST_REPO}" rev-parse main)" '.headRefOid = $stale')
assert_fails_with "origin/top" bash -c 'cd "$1" && bash "$2" managed' _ "${TEST_REPO}" "${preflight}"
export FAKE_PR_12_JSON=${valid_pr_12}

valid_remote_stack=${FAKE_REMOTE_STACK_JSON}
export FAKE_REMOTE_STACK_JSON
FAKE_REMOTE_STACK_JSON=$(printf '%s' "${valid_remote_stack}" | jq -c '.[0].pull_requests[2].state = "closed"')
assert_fails_with "active remote stack order" bash -c 'cd "$1" && bash "$2" managed' _ "${TEST_REPO}" "${preflight}"
FAKE_REMOTE_STACK_JSON=$(printf '%s' "${valid_remote_stack}" | jq -c --arg stale "$(git -C "${TEST_REPO}" rev-parse main)" '.[0].pull_requests[2].head.sha = $stale')
assert_fails_with "remote stack head" bash -c 'cd "$1" && bash "$2" managed' _ "${TEST_REPO}" "${preflight}"
export FAKE_REMOTE_STACK_JSON=${valid_remote_stack}

sibling="${TEST_ROOT}/bottom-worktree"
git -C "${TEST_REPO}" worktree add --quiet "${sibling}" bottom
assert_fails_with "checked out in another worktree" bash -c 'cd "$1" && bash "$2" managed' _ "${TEST_REPO}" "${preflight}"
git -C "${TEST_REPO}" worktree remove "${sibling}"

printf 'local divergence\n' >>"${TEST_REPO}/top.txt"
git -C "${TEST_REPO}" add top.txt
git -C "${TEST_REPO}" commit --quiet -m "local divergence"
remote_clone="${TEST_ROOT}/remote-clone"
git clone --quiet "${TEST_REMOTE}" "${remote_clone}"
git -C "${remote_clone}" config user.email stacked-pr-tests@example.com
git -C "${remote_clone}" config user.name "Stacked PR Tests"
git -C "${remote_clone}" switch --quiet top
printf 'remote divergence\n' >"${remote_clone}/remote.txt"
git -C "${remote_clone}" add remote.txt
git -C "${remote_clone}" commit --quiet -m "remote divergence"
git -C "${remote_clone}" push --quiet origin top
git -C "${TEST_REPO}" fetch --quiet origin top
assert_fails_with "diverged from origin/top" bash -c 'cd "$1" && bash "$2" managed' _ "${TEST_REPO}" "${preflight}"

setup_merged_history_stack merged-history-preflight
merged_snapshot="${TEST_ROOT}/merged-history-before.json"
(cd "${TEST_REPO}" && bash "${preflight}" managed --snapshot "${merged_snapshot}") >/dev/null
[[ $(jq '.branches | length' "${merged_snapshot}") == 1 ]] || fail "expected merged-history snapshot to contain one active branch"
[[ $(jq -r '.branches[0].name' "${merged_snapshot}") == top ]] || fail "expected merged-history snapshot to contain top"
[[ $(jq -r '.branches[0].parentHead' "${merged_snapshot}") == "$(git -C "${TEST_REPO}" rev-parse main)" ]] || fail "expected active top parent to be trunk"

setup_linear_repo rewritten-preflight
setup_fake_stack
rewrite_snapshot="${TEST_ROOT}/rewrite-before.json"
(cd "${TEST_REPO}" && bash "${preflight}" local --snapshot "${rewrite_snapshot}") >/dev/null
printf 'rewritten top\n' >>"${TEST_REPO}/top.txt"
git -C "${TEST_REPO}" add top.txt
git -C "${TEST_REPO}" commit --quiet --amend -m "rewritten top"
setup_fake_stack
assert_fails_with "diverged from origin/top" bash -c 'cd "$1" && bash "$2" local' _ "${TEST_REPO}" "${preflight}"
(cd "${TEST_REPO}" && bash "${preflight}" local --rewrite-snapshot "${rewrite_snapshot}") >/dev/null

main_clone="${TEST_ROOT}/rewritten-main-clone"
git clone --quiet "${TEST_REMOTE}" "${main_clone}"
git -C "${main_clone}" config user.email stacked-pr-tests@example.com
git -C "${main_clone}" config user.name "Stacked PR Tests"
git -C "${main_clone}" switch --quiet main
printf 'advanced trunk\n' >"${main_clone}/advanced-trunk.txt"
git -C "${main_clone}" add advanced-trunk.txt
git -C "${main_clone}" commit --quiet -m "advance trunk"
git -C "${main_clone}" push --quiet origin main
(cd "${TEST_REPO}" && bash "${preflight}" local --rewrite-snapshot "${rewrite_snapshot}") >/dev/null

divergent_tree=$(git -C "${main_clone}" write-tree)
divergent_trunk=$(printf 'rewrite trunk\n' | git -C "${main_clone}" commit-tree "${divergent_tree}")
git -C "${main_clone}" push --quiet origin "${divergent_trunk}:refs/heads/divergent-trunk"
git --git-dir="${TEST_REMOTE}" update-ref refs/heads/main "${divergent_trunk}"
assert_fails_with "trunk history was rewritten" bash -c 'cd "$1" && bash "$2" local --rewrite-snapshot "$3"' _ "${TEST_REPO}" "${preflight}" "${rewrite_snapshot}"
git --git-dir="${TEST_REMOTE}" update-ref refs/heads/main "$(git -C "${main_clone}" rev-parse main)"

remote_clone="${TEST_ROOT}/rewritten-remote-clone"
git clone --quiet "${TEST_REMOTE}" "${remote_clone}"
git -C "${remote_clone}" config user.email stacked-pr-tests@example.com
git -C "${remote_clone}" config user.name "Stacked PR Tests"
git -C "${remote_clone}" switch --quiet top
printf 'unexpected remote movement\n' >"${remote_clone}/unexpected.txt"
git -C "${remote_clone}" add unexpected.txt
git -C "${remote_clone}" commit --quiet -m "unexpected remote movement"
git -C "${remote_clone}" push --quiet origin top
assert_fails_with "moved since the rewrite snapshot" bash -c 'cd "$1" && bash "$2" local --rewrite-snapshot "$3"' _ "${TEST_REPO}" "${preflight}" "${rewrite_snapshot}"

setup_linear_repo rewritten-unpublished-edit
setup_fake_stack
printf 'unpublished edit\n' >>"${TEST_REPO}/top.txt"
git -C "${TEST_REPO}" add top.txt
git -C "${TEST_REPO}" commit --quiet -m "unpublished top edit"
setup_fake_stack
unpublished_snapshot="${TEST_ROOT}/rewrite-unpublished-before.json"
(cd "${TEST_REPO}" && bash "${preflight}" local --snapshot "${unpublished_snapshot}") >/dev/null
[[ $(jq -r '.branches[1].head' "${unpublished_snapshot}") == "$(git -C "${TEST_REPO}" rev-parse top)" ]] || fail "expected rewrite snapshot to retain the unpublished local head"
[[ $(jq -r '.branches[1].remoteHead' "${unpublished_snapshot}") == "$(git -C "${TEST_REPO}" rev-parse origin/top)" ]] || fail "expected rewrite snapshot to retain the published remote head"
git -C "${TEST_REPO}" rebase --quiet --onto main bottom top
setup_fake_stack
assert_fails_with "diverged from origin/top" bash -c 'cd "$1" && bash "$2" local' _ "${TEST_REPO}" "${preflight}"
(cd "${TEST_REPO}" && bash "${preflight}" local --rewrite-snapshot "${unpublished_snapshot}") >/dev/null

setup_linear_repo rewritten-never-published-layer
setup_fake_stack
FAKE_STACK_JSON=$(jq -c '(.branches[] | select(.name == "top")) |= del(.pr)' <<<"${FAKE_STACK_JSON}")
export FAKE_STACK_JSON
git --git-dir="${TEST_REMOTE}" update-ref -d refs/heads/top
never_published_snapshot="${TEST_ROOT}/rewrite-never-published-before.json"
(cd "${TEST_REPO}" && bash "${preflight}" local --snapshot "${never_published_snapshot}") >/dev/null
[[ $(jq -r '.snapshotVersion' "${never_published_snapshot}") == 2 ]] || fail "expected a version 2 rewrite snapshot"
[[ $(jq -r '.branches[] | select(.name == "top") | has("remoteHead")' "${never_published_snapshot}") == false ]] || fail "expected the never-published layer to have no remote lease evidence"
printf 'review rewrite\n' >>"${TEST_REPO}/top.txt"
git -C "${TEST_REPO}" add top.txt
git -C "${TEST_REPO}" commit --quiet --amend -m "reviewed never-published top"
assert_fails_with "stored head for 'top' is not an ancestor" bash -c 'cd "$1" && bash "$2" local' _ "${TEST_REPO}" "${preflight}"
(cd "${TEST_REPO}" && bash "${preflight}" local --rewrite-snapshot "${never_published_snapshot}") >/dev/null
unowned_never_published_snapshot="${TEST_ROOT}/rewrite-never-published-unowned-before.json"
jq --arg unrelated "$(git -C "${TEST_REPO}" rev-parse main)" '(.branches[] | select(.name == "top").head) = $unrelated' "${never_published_snapshot}" >"${unowned_never_published_snapshot}"
assert_fails_with "rewrite snapshot does not own the stored head for 'top'" bash -c 'cd "$1" && bash "$2" local --rewrite-snapshot "$3"' _ "${TEST_REPO}" "${preflight}" "${unowned_never_published_snapshot}"
legacy_never_published_snapshot="${TEST_ROOT}/rewrite-never-published-legacy-before.json"
jq 'del(.snapshotVersion)' "${never_published_snapshot}" >"${legacy_never_published_snapshot}"
(cd "${TEST_REPO}" && bash "${preflight}" local --rewrite-snapshot "${legacy_never_published_snapshot}") >/dev/null
unsupported_snapshot="${TEST_ROOT}/rewrite-unsupported-before.json"
jq '.snapshotVersion = 999' "${never_published_snapshot}" >"${unsupported_snapshot}"
assert_fails_with "rewrite snapshot version is unsupported" bash -c 'cd "$1" && bash "$2" local --rewrite-snapshot "$3"' _ "${TEST_REPO}" "${preflight}" "${unsupported_snapshot}"

setup_linear_repo deleted-published-branch
setup_fake_stack
deleted_branch_snapshot="${TEST_ROOT}/deleted-branch-before.json"
(cd "${TEST_REPO}" && bash "${preflight}" local --snapshot "${deleted_branch_snapshot}") >/dev/null
git --git-dir="${TEST_REMOTE}" update-ref -d refs/heads/top
assert_fails_with "origin/top is missing for published stack layer" bash -c 'cd "$1" && bash "$2" local' _ "${TEST_REPO}" "${preflight}"
assert_fails_with "origin/top is missing since the rewrite snapshot" bash -c 'cd "$1" && bash "$2" local --rewrite-snapshot "$3"' _ "${TEST_REPO}" "${preflight}" "${deleted_branch_snapshot}"
legacy_remote_head_snapshot="${TEST_ROOT}/deleted-branch-legacy-remote-before.json"
jq 'del(.snapshotVersion)' "${deleted_branch_snapshot}" >"${legacy_remote_head_snapshot}"
assert_fails_with "origin/top is missing since the rewrite snapshot" bash -c 'cd "$1" && bash "$2" local --rewrite-snapshot "$3"' _ "${TEST_REPO}" "${preflight}" "${legacy_remote_head_snapshot}"
legacy_deleted_branch_snapshot="${TEST_ROOT}/deleted-branch-legacy-before.json"
jq 'del(.snapshotVersion) | (.branches[] | select(.name == "top")) |= del(.remoteHead)' "${deleted_branch_snapshot}" >"${legacy_deleted_branch_snapshot}"
assert_fails_with "origin/top is missing since the rewrite snapshot" bash -c 'cd "$1" && bash "$2" local --rewrite-snapshot "$3"' _ "${TEST_REPO}" "${preflight}" "${legacy_deleted_branch_snapshot}"

original_remote_url=$(git -C "${TEST_REPO}" config --get remote.origin.url)
git -C "${TEST_REPO}" remote set-url origin "${TEST_ROOT}/unreachable-origin.git"
assert_fails_with "could not query origin/top" bash -c 'cd "$1" && source "$2" && stack_fetch_branch_if_present top' _ "${TEST_REPO}" "${SKILL_DIR}/scripts/common.sh"
git -C "${TEST_REPO}" remote set-url origin "${original_remote_url}"

printf 'PASS: stacked PR preflight contracts\n'
