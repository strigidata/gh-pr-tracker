#!/usr/bin/env bash

set -euo pipefail

test_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

for test_script in base-ref-override.sh preflight.sh validate-stack.sh diff-summary.sh safe-sync.sh; do
  printf 'Running %s\n' "${test_script}"
  bash "${test_dir}/${test_script}"
done

printf 'PASS: all stacked PR workflow contracts\n'
