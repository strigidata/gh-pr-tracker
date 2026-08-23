#!/usr/bin/env bash

# Shared fail-closed helpers for the native stacked PR workflow.

STACK_GH_BIN=${GH_BIN:-gh}
STACK_REMOTE=origin
STACK_SNAPSHOT_VERSION=2

# Print an actionable error and stop the current script.
stack_die() {
  printf 'ERROR: %s\n' "$*" >&2
  exit 1
}

# Require a command used by the guard scripts.
stack_require_command() {
  command -v "$1" >/dev/null 2>&1 || stack_die "required command '$1' is unavailable"
}

# Remove a release tag's optional v prefix.
stack_normalize_version() {
  printf '%s' "${1#v}"
}

# Reject snapshots from a schema this checkout cannot interpret safely.
stack_validate_snapshot_version() {
  local snapshot=$1

  jq -e --argjson current "${STACK_SNAPSHOT_VERSION}" \
    '(.snapshotVersion // 1) | type == "number" and . == floor and . >= 1 and . <= $current' \
    "${snapshot}" >/dev/null || stack_die "rewrite snapshot version is unsupported"
}

# Return the captured published head. Unversioned snapshots may carry remoteHead.
stack_snapshot_published_head() {
  local snapshot=$1 branch=$2 legacy_published=${3:-false}

  jq -er \
    --arg branch "${branch}" \
    --argjson current "${STACK_SNAPSHOT_VERSION}" \
    --argjson legacyPublished "${legacy_published}" \
    '(.snapshotVersion // 1) as $version
      | .branches[]
      | select(.name == $branch)
      | if $version == $current then .remoteHead
        elif .remoteHead != null then .remoteHead
        elif $legacyPublished then .head
        else empty
        end' \
    "${snapshot}"
}

# Verify the installed GitHub tools exactly match their latest stable releases.
stack_check_tool_versions() {
  local gh_installed stack_installed gh_latest stack_latest

  gh_installed=$("${STACK_GH_BIN}" --version | awk 'NR == 1 {print $3}')
  stack_installed=$("${STACK_GH_BIN}" stack --version | awk 'NR == 1 {print $4}')
  if ! gh_latest=$("${STACK_GH_BIN}" api repos/cli/cli/releases/latest --jq .tag_name); then
    stack_die "could not confirm the latest stable gh version"
  fi
  if ! stack_latest=$("${STACK_GH_BIN}" api repos/github/gh-stack/releases/latest --jq .tag_name); then
    stack_die "could not confirm the latest stable gh-stack version"
  fi
  gh_latest=$(stack_normalize_version "${gh_latest}")
  stack_latest=$(stack_normalize_version "${stack_latest}")
  [[ ${gh_installed} == "${gh_latest}" ]] || stack_die "gh ${gh_installed} is stale; latest stable is ${gh_latest}"
  [[ ${stack_installed} == "${stack_latest}" ]] || stack_die "gh-stack ${stack_installed} is stale; latest stable is ${stack_latest}"
  printf 'Tools: gh %s, gh-stack %s\n' "${gh_installed}" "${stack_installed}"
}

# Require authenticated GitHub CLI access.
stack_check_authentication() {
  "${STACK_GH_BIN}" auth status >/dev/null 2>&1 || stack_die "GitHub CLI authentication is unavailable"
}

# Extract the owner and repository from a GitHub remote URL.
stack_repository_from_url() {
  local remote_url=$1 repository

  case ${remote_url} in
    https://github.com/*) repository=${remote_url#https://github.com/} ;;
    git@github.com:*) repository=${remote_url#git@github.com:} ;;
    ssh://git@github.com/*) repository=${remote_url#ssh://git@github.com/} ;;
    *) stack_die "${STACK_REMOTE} remote is not a github.com repository: ${remote_url}" ;;
  esac
  repository=${repository%.git}
  [[ ${repository} == */* && ${repository#*/} != */* ]] || stack_die "could not resolve the ${STACK_REMOTE} repository from ${remote_url}"
  printf '%s\n' "${repository}"
}

# Return the repository owned by the fixed stack remote.
stack_remote_repository() {
  local fetch_url push_url fetch_repository push_repository

  fetch_url=$(git config --get "remote.${STACK_REMOTE}.url") || stack_die "remote '${STACK_REMOTE}' does not exist"
  push_url=$(git config --get "remote.${STACK_REMOTE}.pushurl" || printf '%s' "${fetch_url}")
  fetch_repository=$(stack_repository_from_url "${fetch_url}")
  push_repository=$(stack_repository_from_url "${push_url}")
  [[ ${fetch_repository} == "${push_repository}" ]] || stack_die "${STACK_REMOTE} fetch and push repositories disagree"
  printf '%s\n' "${fetch_repository}"
}

# Require gh's current repository to match the fixed stack remote.
stack_check_repository_identity() {
  local remote_repository gh_repository

  remote_repository=$(stack_remote_repository)
  gh_repository=$("${STACK_GH_BIN}" repo view --json nameWithOwner --jq .nameWithOwner) || stack_die "could not resolve GitHub repository identity"
  [[ ${gh_repository} == "${remote_repository}" ]] || stack_die "${STACK_REMOTE} repository '${remote_repository}' does not match GitHub repository '${gh_repository}'"
  printf 'Repository: %s via %s\n' "${remote_repository}" "${STACK_REMOTE}"
}

# Refresh gh-stack metadata once, then return the persisted view from a second read.
stack_read_refreshed_view() {
  local stack_json

  "${STACK_GH_BIN}" stack view --json >/dev/null || stack_die "current branch is not in an unambiguous local stack"
  stack_json=$("${STACK_GH_BIN}" stack view --json) || stack_die "current branch is not in an unambiguous local stack"
  printf '%s\n' "${stack_json}"
}

# Return a stack view containing only branches whose pull requests remain active.
stack_active_view() {
  local stack_json=$1

  jq -c '.branches |= map(select((.isMerged // false) == false and ((.pr.state // "OPEN") != "MERGED")))' <<<"${stack_json}"
}

# Require a clean current worktree.
stack_check_clean_worktree() {
  [[ -z $(git status --porcelain) ]] || stack_die "working tree is not clean"
}

# Return success when a Git or gh-stack recovery path exists.
stack_has_active_operation() {
  local name path

  for name in rebase-merge rebase-apply MERGE_HEAD CHERRY_PICK_HEAD REVERT_HEAD sequencer gh-stack-rebase-state gh-stack-modify-state; do
    path=$(git rev-parse --git-path "${name}")
    [[ -e ${path} ]] && return 0
  done
  [[ -n $(git ls-files -u) ]]
}

# Reject active Git operations and stack recovery state before new mutation.
stack_check_no_active_operation() {
  if stack_has_active_operation; then
    stack_die "a Git operation or gh-stack recovery state is active"
  fi
  return 0
}

# Require a local branch or commit ref.
stack_check_ref() {
  git rev-parse --verify --quiet "$1^{commit}" >/dev/null || stack_die "ref '$1' does not exist"
}

# Reject a stack branch checked out outside the current worktree.
stack_check_branch_worktree() {
  local target=$1 current_root worktree_path worktree_branch

  current_root=$(git rev-parse --show-toplevel)
  worktree_path=''
  while IFS= read -r line; do
    case ${line} in
      worktree\ *) worktree_path=${line#worktree } ;;
      branch\ *)
        worktree_branch=${line#branch refs/heads/}
        if [[ ${worktree_branch} == "${target}" && ${worktree_path} != "${current_root}" ]]; then
          stack_die "branch '${target}' is checked out in another worktree: ${worktree_path}"
        fi
        ;;
    esac
  done < <(git worktree list --porcelain)
}

# Reject unexplained two-way divergence while allowing a snapshot-proven rewrite.
stack_check_divergence() {
  local branch=$1 rewrite_snapshot=${2:-} stored_head=${3:-} trunk_ref=${4:-} legacy_published=${5:-false}
  local ahead behind counts captured_head captured_trunk remote_head actual_head current_trunk

  if ! git rev-parse --verify --quiet "${STACK_REMOTE}/${branch}^{commit}" >/dev/null; then
    if [[ -n ${rewrite_snapshot} ]] && stack_snapshot_published_head "${rewrite_snapshot}" "${branch}" "${legacy_published}" >/dev/null 2>&1; then
      stack_die "${STACK_REMOTE}/${branch} is missing since the rewrite snapshot"
    fi
    [[ ${legacy_published} == false ]] || stack_die "${STACK_REMOTE}/${branch} is missing for published stack layer"
    return 0
  fi
  counts=$(git rev-list --left-right --count "${STACK_REMOTE}/${branch}...${branch}")
  behind=${counts%%[[:space:]]*}
  ahead=${counts##*[[:space:]]}
  if ((behind > 0 && ahead > 0)); then
    [[ -n ${rewrite_snapshot} ]] || stack_die "branch '${branch}' diverged from ${STACK_REMOTE}/${branch}"
    captured_head=$(stack_snapshot_published_head "${rewrite_snapshot}" "${branch}" "${legacy_published}") || stack_die "rewrite snapshot has no published head for '${branch}'"
    captured_trunk=$(jq -er '.trunkHead' "${rewrite_snapshot}") || stack_die "rewrite snapshot has no trunk head"
    remote_head=$(git rev-parse "${STACK_REMOTE}/${branch}")
    actual_head=$(git rev-parse "${branch}")
    current_trunk=$(git rev-parse "${trunk_ref}")
    [[ ${remote_head} == "${captured_head}" ]] || stack_die "${STACK_REMOTE}/${branch} moved since the rewrite snapshot"
    git merge-base --is-ancestor "${captured_trunk}" "${current_trunk}" || stack_die "trunk history was rewritten since the rewrite snapshot"
    if [[ ${stored_head} != "${actual_head}" ]] && ! git merge-base --is-ancestor "${stored_head}" "${actual_head}"; then
      stack_die "local rewrite for '${branch}' is not tracked by current stack metadata"
    fi
    printf "Branch %s: verified rewrite pending publication from captured remote %s\n" "${branch}" "${remote_head}"
    return 0
  fi
  printf 'Branch %s: local ahead %s, remote ahead %s\n' "${branch}" "${ahead}" "${behind}"
}

# Fetch a remote stack branch when it exists without treating an unpublished branch as an error.
stack_fetch_branch_if_present() {
  local branch=$1 remote_status

  if git ls-remote --exit-code --heads "${STACK_REMOTE}" "${branch}" >/dev/null 2>&1; then
    git fetch --no-tags --quiet "${STACK_REMOTE}" "${branch}"
  else
    remote_status=$?
    if ((remote_status == 2)); then
      git update-ref -d "refs/remotes/${STACK_REMOTE}/${branch}"
      return 0
    fi
    stack_die "could not query ${STACK_REMOTE}/${branch}"
  fi
}

# Require a local branch to equal its tracking head after publication.
stack_check_published_head() {
  local branch=$1 tracking_head=$2 require_published=$3

  if [[ ${require_published} == true && $(git rev-parse "${branch}") != "${tracking_head}" ]]; then
    stack_die "branch '${branch}' is not published to ${STACK_REMOTE}/${branch}"
  fi
}

# Require one live pull request to match its local stack member.
stack_check_live_pr() {
  local stack_json=$1 index=$2 repository=$3 require_published=$4
  local branch expected_base pr_number pr_json remote_head state

  branch=$(jq -r ".branches[${index}].name" <<<"${stack_json}")
  expected_base=$(jq -r --argjson index "${index}" 'if $index == 0 then .trunk else .branches[$index - 1].name end' <<<"${stack_json}")
  pr_number=$(jq -r ".branches[${index}].pr.number // empty" <<<"${stack_json}")
  [[ -n ${pr_number} ]] || stack_die "branch '${branch}' has no pull request in a managed stack"
  pr_json=$("${STACK_GH_BIN}" pr view "${pr_number}" --repo "${repository}" --json number,state,baseRefName,headRefName,headRefOid)
  remote_head=$(git rev-parse "${STACK_REMOTE}/${branch}")
  state=$(jq -r '.state' <<<"${pr_json}")
  [[ ${state} == OPEN ]] || stack_die "PR #${pr_number} is ${state}; expected OPEN"
  [[ $(jq -r '.headRefName' <<<"${pr_json}") == "${branch}" ]] || stack_die "PR #${pr_number} head branch does not match '${branch}'"
  [[ $(jq -r '.baseRefName' <<<"${pr_json}") == "${expected_base}" ]] || stack_die "PR #${pr_number} base branch does not match '${expected_base}'"
  [[ $(jq -r '.headRefOid' <<<"${pr_json}") == "${remote_head}" ]] || stack_die "PR #${pr_number} head SHA does not match ${STACK_REMOTE}/${branch}"
  stack_check_published_head "${branch}" "${remote_head}" "${require_published}"
}

# Require the pull-request chain to match local stack order.
stack_check_live_pr_chain() {
  local stack_json=$1 require_published=${2:-false} repository count index

  stack_json=$(stack_active_view "${stack_json}")
  repository=$(stack_remote_repository)
  count=$(jq '.branches | length' <<<"${stack_json}")
  for ((index = 0; index < count; index++)); do
    stack_check_live_pr "${stack_json}" "${index}" "${repository}" "${require_published}"
  done
}

# Require one active remote-stack member to match its tracking ref.
stack_check_remote_stack_member() {
  local stack_json=$1 active_prs=$2 index=$3 require_published=$4
  local branch pr_number tracking_sha

  branch=$(jq -r ".branches[${index}].name" <<<"${stack_json}")
  pr_number=$(jq -r ".branches[${index}].pr.number" <<<"${stack_json}")
  tracking_sha=$(git rev-parse "${STACK_REMOTE}/${branch}")
  [[ $(jq -r ".[${index}].state" <<<"${active_prs}") == open ]] || stack_die "remote stack PR #${pr_number} is not open"
  [[ $(jq -r ".[${index}].head.ref" <<<"${active_prs}") == "${branch}" ]] || stack_die "remote stack PR #${pr_number} head does not match '${branch}'"
  [[ $(jq -r ".[${index}].head.sha" <<<"${active_prs}") == "${tracking_sha}" ]] || stack_die "remote stack head for '${branch}' does not match ${STACK_REMOTE}/${branch}"
  stack_check_published_head "${branch}" "${tracking_sha}" "${require_published}"
}

# Require GitHub's remote stack object to contain the same pull requests in order.
stack_check_remote_stack() {
  local stack_json=$1 require_published=${2:-false} repository expected_prs first_pr remote_json active_prs remote_prs remote_base trunk
  local count remote_count index

  stack_json=$(stack_active_view "${stack_json}")
  repository=$(stack_remote_repository)
  expected_prs=$(jq -c '[.branches[].pr.number]' <<<"${stack_json}")
  count=$(jq '.branches | length' <<<"${stack_json}")
  first_pr=$(jq -r '.[0]' <<<"${expected_prs}")
  remote_json=$("${STACK_GH_BIN}" api "repos/${repository}/stacks?pull_request=${first_pr}")
  jq -e 'type == "array"' <<<"${remote_json}" >/dev/null || stack_die "GitHub remote stack response must be an array"
  remote_count=$(jq 'length' <<<"${remote_json}")
  if ((count == 1 && remote_count == 0)); then
    printf 'GitHub has no remote stack object for the one-layer PR; live PR metadata is authoritative\n'
    return 0
  fi
  [[ ${remote_count} -eq 1 ]] || stack_die "GitHub remote stack metadata is missing or ambiguous"
  active_prs=$(jq -c '.[0].pull_requests | map(select(.state == "open"))' <<<"${remote_json}")
  remote_prs=$(jq -c 'map(.number)' <<<"${active_prs}")
  [[ ${remote_prs} == "${expected_prs}" ]] || stack_die "local and GitHub active remote stack order disagree"
  remote_base=$(jq -r '.[0].base.ref' <<<"${remote_json}")
  trunk=$(jq -r '.trunk' <<<"${stack_json}")
  [[ ${remote_base} == "${trunk}" ]] || stack_die "GitHub remote stack base '${remote_base}' does not match '${trunk}'"
  for ((index = 0; index < count; index++)); do
    stack_check_remote_stack_member "${stack_json}" "${active_prs}" "${index}" "${require_published}"
  done
}

# Serialize current branch and parent heads for post-cascade validation.
stack_write_snapshot() {
  local output=$1 trunk_ref=$2
  shift 2
  local records='[]' parent_ref=${trunk_ref} branch branch_head parent_head remote_head payload

  for branch in "$@"; do
    branch_head=$(git rev-parse "${branch}")
    parent_head=$(git rev-parse "${parent_ref}")
    remote_head=''
    if git rev-parse --verify --quiet "${STACK_REMOTE}/${branch}^{commit}" >/dev/null; then
      remote_head=$(git rev-parse "${STACK_REMOTE}/${branch}")
    fi
    records=$(jq -nc --argjson records "${records}" --arg name "${branch}" --arg head "${branch_head}" --arg parent "${parent_head}" --arg remote "${remote_head}" '$records + [({name:$name,head:$head,parentHead:$parent} + (if $remote == "" then {} else {remoteHead:$remote} end))]')
    parent_ref=${branch}
  done
  payload=$(jq -nc --argjson version "${STACK_SNAPSHOT_VERSION}" --arg trunk "$(git rev-parse "${trunk_ref}")" --argjson branches "${records}" '{snapshotVersion:$version,trunkHead:$trunk,branches:$branches}')
  printf '%s\n' "${payload}" >"${output}"
  printf 'Snapshot: %s\n' "${output}"
}
