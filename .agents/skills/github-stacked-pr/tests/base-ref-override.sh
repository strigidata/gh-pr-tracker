#!/usr/bin/env bash

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/helpers.sh"

setup_linear_repo base-ref
base_ref_script="${REPO_ROOT}/.agents/skills/code-review/lib/base-ref.sh"

actual=$(cd "${TEST_REPO}" && CR_BASE_REF_OVERRIDE=bottom bash -c 'source "$1"; printf "%s" "$BASE_REF"' _ "${base_ref_script}")
[[ ${actual} == bottom ]] || fail "valid override selected '${actual}' instead of 'bottom'"

actual=$(cd "${TEST_REPO}" && CR_BASE_REF_OVERRIDE=bottom zsh -c 'source "$1"; printf "%s:%s" "$BASE_REF" "$(git rev-parse --short HEAD)"' _ "${base_ref_script}")
[[ ${actual} == bottom:* ]] || fail "zsh override lost its command path: '${actual}'"

assert_fails_with "does not exist" bash -c 'cd "$1" && CR_BASE_REF_OVERRIDE=missing source "$2"' _ "${TEST_REPO}" "${base_ref_script}"

git -C "${TEST_REPO}" switch --quiet -c side main
printf 'side\n' >"${TEST_REPO}/side.txt"
git -C "${TEST_REPO}" add side.txt
git -C "${TEST_REPO}" commit --quiet -m side
git -C "${TEST_REPO}" switch --quiet top
assert_fails_with "not an ancestor" bash -c 'cd "$1" && CR_BASE_REF_OVERRIDE=side source "$2"' _ "${TEST_REPO}" "${base_ref_script}"

actual=$(cd "${TEST_REPO}" && env -u CR_BASE_REF_OVERRIDE bash -c 'source "$1"; printf "%s" "$BASE_REF"' _ "${base_ref_script}")
[[ ${actual} == origin/top ]] || fail "standalone fallback changed to '${actual}'"

# A remote-tracking override must be refetched, or cumulative stack verification
# runs against whatever origin/main happened to be at the last local fetch.
setup_linear_repo base-ref-stale
expected=$(git -C "${TEST_REPO}" rev-parse bottom)
git --git-dir="${TEST_REMOTE}" update-ref refs/heads/main "${expected}"

actual=$(cd "${TEST_REPO}" && CR_BASE_REF_OVERRIDE=origin/main bash -c 'source "$1"; git rev-parse "$BASE_REF"' _ "${base_ref_script}")
[[ ${actual} == "${expected}" ]] || fail "stale override resolved '${actual}' instead of refreshed '${expected}'"

# A local override that merely looks path-like must not be treated as a remote.
actual=$(cd "${TEST_REPO}" && CR_BASE_REF_OVERRIDE=refs/heads/bottom bash -c 'source "$1"; git rev-parse "$BASE_REF"' _ "${base_ref_script}")
[[ ${actual} == "${expected}" ]] || fail "local path-like override resolved '${actual}' instead of '${expected}'"

printf 'PASS: base-ref override contracts\n'
