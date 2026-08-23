#!/usr/bin/env bash

set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
source "${script_dir}/common.sh"

# Run sync only when the extension cannot offer destructive interactive choices.
if (($#)); then
  printf 'Usage: %s\n' "$0" >&2
  exit 2
fi
if [[ -t 0 || -t 1 || -t 2 ]]; then
  stack_die "safe sync requires non-interactive input and output"
fi

stack_require_command "${STACK_GH_BIN}"
stack_check_authentication
git rev-parse --show-toplevel >/dev/null 2>&1 || stack_die "not inside a Git repository"
stack_check_repository_identity
stack_check_clean_worktree
stack_check_no_active_operation

"${STACK_GH_BIN}" stack sync --remote "${STACK_REMOTE}"
