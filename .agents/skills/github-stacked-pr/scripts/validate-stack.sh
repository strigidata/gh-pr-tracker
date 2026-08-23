#!/usr/bin/env bash

set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
source "${script_dir}/common.sh"

# Print validator arguments.
usage() {
  printf 'Usage: %s [--local | --published] --trunk SHA [--before SNAPSHOT] BRANCH...\n' "$0" >&2
  exit 2
}

# Parse a captured trunk SHA, optional before snapshot, and ordered branches.
parse_arguments() {
  trunk_sha=''
  before_path=''
  require_remote=true
  require_published=false
  branches=()
  while (($#)); do
    case $1 in
      --local) require_remote=false; shift ;;
      --published) require_published=true; shift ;;
      --trunk) [[ $# -ge 2 ]] || usage; trunk_sha=$2; shift 2 ;;
      --before) [[ $# -ge 2 ]] || usage; before_path=$2; shift 2 ;;
      --*) usage ;;
      *) branches+=("$1"); shift ;;
    esac
  done
  [[ -n ${trunk_sha} && ${#branches[@]} -ge 1 ]] || usage
  if [[ ${require_remote} == false && ${require_published} == true ]]; then usage; fi
}

# Detect a child that remained unchanged after its recorded parent moved.
check_expected_movement() {
  local branch=$1 parent_ref=$2 before_parent before_head current_parent current_head

  [[ -n ${before_path} ]] || return 0
  before_parent=$(jq -r --arg branch "${branch}" '.branches[] | select(.name == $branch) | .parentHead' "${before_path}")
  before_head=$(jq -r --arg branch "${branch}" '.branches[] | select(.name == $branch) | .head' "${before_path}")
  [[ -n ${before_parent} && -n ${before_head} ]] || stack_die "before snapshot has no record for '${branch}'"
  current_parent=$(git rev-parse "${parent_ref}")
  current_head=$(git rev-parse "${branch}")
  if [[ ${current_parent} != "${before_parent}" && ${current_head} == "${before_head}" ]]; then
    stack_die "branch '${branch}' did not move after parent '${parent_ref}' changed"
  fi
}

# Require current refs to form the expected ancestry chain.
check_ancestry() {
  local parent_ref=${trunk_sha} branch

  stack_check_ref "${trunk_sha}"
  for branch in "${branches[@]}"; do
    stack_check_ref "${branch}"
    check_expected_movement "${branch}" "${parent_ref}"
    git merge-base --is-ancestor "${parent_ref}" "${branch}" || stack_die "'${parent_ref}' is not an ancestor of '${branch}'"
    parent_ref=${branch}
  done
}

# Reject unresolved conflict markers and malformed JSON in every layer tree.
# A cascade that stops mid-conflict can leave a layer committed with markers in
# it, and ancestry validation alone will not notice.
check_layer_artifacts() {
  local parent_ref=${trunk_sha} branch file_path blob

  for branch in "${branches[@]}"; do
    while IFS= read -r -d '' file_path; do
      blob=$(git show "${branch}:${file_path}" 2>/dev/null) || continue

      if printf '%s\n' "${blob}" | grep -qE '^(<{7}|={7}|>{7})( |$)'; then
        stack_die "unresolved conflict marker in '${file_path}' on branch '${branch}'"
      fi

      case ${file_path} in
        *.json)
          printf '%s\n' "${blob}" | jq empty >/dev/null 2>&1 ||
            stack_die "invalid JSON '${file_path}' in branch '${branch}'"
          ;;
      esac
    done < <(
      git diff --name-only -z --diff-filter=ACMR "${parent_ref}" "${branch}"
    )
    parent_ref=${branch}
  done
}

# Require local gh-stack metadata to match the supplied order and current heads.
check_local_metadata() {
  local stack_json expected_names actual_names count index branch stored_head actual_head needs_rebase

  stack_json=$(stack_read_refreshed_view)
  stack_json=$(stack_active_view "${stack_json}")
  expected_names=$(printf '%s\n' "${branches[@]}" | jq -Rsc 'split("\n") | map(select(length > 0))')
  actual_names=$(jq -c '[.branches[].name]' <<<"${stack_json}")
  [[ ${actual_names} == "${expected_names}" ]] || stack_die "local stack order does not match the expected branch order"
  count=${#branches[@]}
  index=0
  while ((index < count)); do
    branch=${branches[index]}
    stored_head=$(jq -r ".branches[${index}].head // empty" <<<"${stack_json}")
    actual_head=$(git rev-parse "${branch}")
    [[ ${stored_head} == "${actual_head}" ]] || stack_die "local stack metadata head for '${branch}' does not match its ref"
    needs_rebase=$(jq -r ".branches[${index}].needsRebase" <<<"${stack_json}")
    [[ ${needs_rebase} == false ]] || stack_die "local stack metadata still marks '${branch}' as needing rebase"
    index=$((index + 1))
  done
  if [[ ${require_remote} == true ]]; then
    stack_check_repository_identity
    stack_check_live_pr_chain "${stack_json}" "${require_published}"
    stack_check_remote_stack "${stack_json}" "${require_published}"
  fi
}

parse_arguments "$@"
stack_require_command git
stack_require_command jq
check_ancestry
check_layer_artifacts
check_local_metadata
printf 'Stack topology validated: %s\n' "${branches[*]}"
