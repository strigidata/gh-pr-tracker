#!/usr/bin/env bash

set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
source "${script_dir}/common.sh"

# Print the supported preflight modes and arguments.
usage() {
  printf 'Usage: %s adopt [--snapshot PATH] BRANCH...\n' "$0" >&2
  printf '       %s local|managed [--snapshot PATH] [--rewrite-snapshot PATH]\n' "$0" >&2
  exit 2
}

# Parse a snapshot option while preserving ordered branch arguments.
parse_arguments() {
  [[ $# -ge 1 ]] || usage
  mode=$1
  shift
  snapshot_path=''
  rewrite_snapshot_path=''
  branches=()
  while (($#)); do
    case $1 in
      --snapshot)
        [[ $# -ge 2 ]] || usage
        snapshot_path=$2
        shift 2
        ;;
      --rewrite-snapshot)
        [[ $# -ge 2 ]] || usage
        rewrite_snapshot_path=$2
        shift 2
        ;;
      --*) usage ;;
      *) branches+=("$1"); shift ;;
    esac
  done
  [[ ${mode} == adopt || ${mode} == local || ${mode} == managed ]] || usage
  if [[ ${mode} == adopt && -n ${rewrite_snapshot_path} ]]; then usage; fi
  if [[ -n ${rewrite_snapshot_path} ]]; then
    [[ -r ${rewrite_snapshot_path} ]] || stack_die "rewrite snapshot is not readable"
    jq empty "${rewrite_snapshot_path}" >/dev/null 2>&1 || stack_die "rewrite snapshot is invalid JSON"
    stack_validate_snapshot_version "${rewrite_snapshot_path}"
  fi
  if [[ -n ${snapshot_path} && ${snapshot_path} == "${rewrite_snapshot_path}" ]]; then
    stack_die "output snapshot must not overwrite the rewrite snapshot"
  fi
}

# Fetch the trunk and any already-published stack branches.
fetch_stack_refs() {
  local trunk=$1
  shift

  git fetch --no-tags --quiet "${STACK_REMOTE}" "${trunk}" || stack_die "could not fetch ${STACK_REMOTE}/${trunk}"
  for branch in "$@"; do
    stack_fetch_branch_if_present "${branch}"
  done
}

# Check local refs, worktree ownership, and remote divergence for each layer.
check_stack_branches() {
  local stack_json=$1 trunk_ref=$2
  shift 2
  local branch stored_head pr_number legacy_published

  for branch in "$@"; do
    stack_check_ref "${branch}"
    stack_check_branch_worktree "${branch}"
    stored_head=''
    if [[ -n ${stack_json} ]]; then
      stored_head=$(jq -r --arg branch "${branch}" '.branches[] | select(.name == $branch) | .head // empty' <<<"${stack_json}")
    fi
    pr_number=''
    if [[ -n ${stack_json} ]]; then
      pr_number=$(jq -r --arg branch "${branch}" '.branches[] | select(.name == $branch) | .pr.number // empty' <<<"${stack_json}")
    fi
    legacy_published=false
    [[ -n ${pr_number} ]] && legacy_published=true
    stack_check_divergence "${branch}" "${rewrite_snapshot_path}" "${stored_head}" "${trunk_ref}" "${legacy_published}"
  done
}

# Require every adopted layer to share history with its declared parent.
check_adoption_merge_bases() {
  local parent=$1 branch
  shift

  for branch in "$@"; do
    if ! git merge-base "${parent}" "${branch}" >/dev/null; then
      stack_die "branch '${branch}' has no merge base with '${parent}'"
    fi
    if ! git merge-base --is-ancestor "${parent}" "${branch}"; then
      printf "Adoption note: '%s' requires cascading rebase onto '%s'.\n" "${branch}" "${parent}"
    fi
    parent=${branch}
  done
}

# Verify stored stack heads have not been replaced by unrelated local history.
check_local_metadata_heads() {
  local stack_json=$1 rewrite_snapshot=${2:-} count index branch stored_head actual_head captured_head

  count=$(jq '.branches | length' <<<"${stack_json}")
  index=0
  while ((index < count)); do
    branch=$(jq -r ".branches[${index}].name" <<<"${stack_json}")
    stored_head=$(jq -r ".branches[${index}].head // empty" <<<"${stack_json}")
    actual_head=$(git rev-parse "${branch}")
    if [[ -n ${stored_head} ]] && ! git merge-base --is-ancestor "${stored_head}" "${actual_head}"; then
      [[ -n ${rewrite_snapshot} ]] || stack_die "stored head for '${branch}' is not an ancestor of its local ref"
      captured_head=$(jq -er --arg branch "${branch}" '.branches[] | select(.name == $branch) | .head' "${rewrite_snapshot}") || stack_die "rewrite snapshot has no local head for '${branch}'"
      [[ ${captured_head} == "${stored_head}" ]] || stack_die "rewrite snapshot does not own the stored head for '${branch}'"
    fi
    index=$((index + 1))
  done
}

# Run preflight for branches that have not yet been adopted into local metadata.
run_adopt_preflight() {
  local trunk=main

  ((${#branches[@]} >= 1)) || usage
  fetch_stack_refs "${trunk}" "${branches[@]}"
  check_stack_branches '' "origin/${trunk}" "${branches[@]}"
  check_adoption_merge_bases "origin/${trunk}" "${branches[@]}"
  if [[ -n ${snapshot_path} ]]; then
    stack_write_snapshot "${snapshot_path}" "origin/${trunk}" "${branches[@]}"
  fi
  printf 'Adoption preflight passed: %s\n' "${branches[*]}"
}

# Run preflight for an existing local or GitHub-managed stack.
run_existing_preflight() {
  local stack_json active_stack_json trunk

  ((${#branches[@]} == 0)) || usage
  stack_json=$(stack_read_refreshed_view)
  trunk=$(jq -r '.trunk' <<<"${stack_json}")
  active_stack_json=$(stack_active_view "${stack_json}")
  while IFS= read -r branch; do branches+=("${branch}"); done < <(jq -r '.branches[].name' <<<"${active_stack_json}")
  ((${#branches[@]} >= 1)) || stack_die "local stack has no active branches"
  fetch_stack_refs "${trunk}" "${branches[@]}"
  check_stack_branches "${active_stack_json}" "origin/${trunk}" "${branches[@]}"
  check_local_metadata_heads "${active_stack_json}" "${rewrite_snapshot_path}"
  if [[ ${mode} == managed ]]; then
    stack_check_live_pr_chain "${active_stack_json}"
    stack_check_remote_stack "${active_stack_json}"
  fi
  if [[ -n ${snapshot_path} ]]; then
    stack_write_snapshot "${snapshot_path}" "origin/${trunk}" "${branches[@]}"
  fi
  printf '%s stack preflight passed: %s\n' "${mode}" "${branches[*]}"
}

parse_arguments "$@"
stack_require_command git
stack_require_command jq
stack_check_tool_versions
stack_check_authentication
git rev-parse --show-toplevel >/dev/null 2>&1 || stack_die "not inside a Git repository"
stack_check_repository_identity
stack_check_clean_worktree
stack_check_no_active_operation

if [[ ${mode} == adopt ]]; then
  run_adopt_preflight
else
  run_existing_preflight
fi
