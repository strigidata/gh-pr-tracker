#!/usr/bin/env bash

set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
source "${script_dir}/common.sh"

# Print diff-summary arguments.
usage() {
  printf 'Usage: %s --trunk REF BRANCH...\n' "$0" >&2
  exit 2
}

# Print commit and file counts for one validated diff range.
print_range() {
  local label=$1 base=$2 head=$3 commits files

  stack_check_ref "${base}"
  stack_check_ref "${head}"
  git merge-base --is-ancestor "${base}" "${head}" || stack_die "'${base}' is not an ancestor of '${head}'"
  commits=$(git rev-list --count "${base}..${head}")
  files=$(git diff --name-only "${base}...${head}" | wc -l | tr -d ' ')
  printf '%s: %s...%s (%s commits, %s files)\n' "${label}" "${base}" "${head}" "${commits}" "${files}"
}

trunk=''
branches=()
while (($#)); do
  case $1 in
    --trunk) [[ $# -ge 2 ]] || usage; trunk=$2; shift 2 ;;
    --*) usage ;;
    *) branches+=("$1"); shift ;;
  esac
done
[[ -n ${trunk} && ${#branches[@]} -ge 1 ]] || usage

parent=${trunk}
for branch in "${branches[@]}"; do
  print_range "Layer ${branch}" "${parent}" "${branch}"
  parent=${branch}
done
top=${branches[$((${#branches[@]} - 1))]}
print_range "Cumulative ${top}" "${trunk}" "${top}"
