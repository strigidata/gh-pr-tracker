#!/usr/bin/env bash

set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/helpers.sh"

safe_sync="${SKILL_DIR}/scripts/safe-sync.sh"
setup_linear_repo safe-sync
setup_fake_stack

invocation_file="${TEST_ROOT}/sync-invocation"
export FAKE_GH_INVOCATION_FILE=${invocation_file}
sync_output="${TEST_ROOT}/sync-output"
(cd "${TEST_REPO}" && bash "${safe_sync}") </dev/null >"${sync_output}" 2>&1

[[ $(<"${invocation_file}") == 'stack sync --remote origin' ]] || fail "safe sync did not pin the origin remote"

# The wrapper rejects a TTY on any stream, so a documented invocation that keeps
# one attached cannot be followed as written.
documented=$(grep -n 'bash .*scripts/safe-sync\.sh' "${SKILL_DIR}/SKILL.md" || true)
[[ -n ${documented} ]] || fail "SKILL.md no longer documents the safe sync wrapper"
while IFS= read -r line; do
  [[ ${line} == *'</dev/null'* ]] || fail "documented safe sync invocation keeps stdin interactive: ${line}"
  [[ ${line} == *'2>&1'* ]] || fail "documented safe sync invocation keeps output interactive: ${line}"
done <<<"${documented}"

printf 'PASS: safe non-interactive sync contract\n'
