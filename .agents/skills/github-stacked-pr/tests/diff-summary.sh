#!/usr/bin/env bash

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/helpers.sh"

summary_script="${SKILL_DIR}/scripts/diff-summary.sh"
setup_linear_repo diff-summary

output=$(cd "${TEST_REPO}" && bash "${summary_script}" --trunk origin/main bottom top)
assert_contains "${output}" "Layer bottom: origin/main...bottom"
assert_contains "${output}" "Layer top: bottom...top"
assert_contains "${output}" "Cumulative top: origin/main...top"

git -C "${TEST_REPO}" switch --quiet -c side main
printf 'side\n' >"${TEST_REPO}/side.txt"
git -C "${TEST_REPO}" add side.txt
git -C "${TEST_REPO}" commit --quiet -m side
git -C "${TEST_REPO}" switch --quiet top
assert_fails_with "not an ancestor" bash -c 'cd "$1" && bash "$2" --trunk origin/main bottom side' _ "${TEST_REPO}" "${summary_script}"

printf 'PASS: diff summary contracts\n'
