#!/usr/bin/env bash
# Validate that a code-review playbook has the required sections.
# Usage: validate-playbook.sh <path-to-playbook.md>
# Exit 0 = valid, exit 1 = invalid (with reasons to stderr).

set -euo pipefail

if [ $# -ne 1 ]; then
  echo "usage: $0 <playbook.md>" >&2
  exit 1
fi

file="$1"
if [ ! -f "$file" ]; then
  echo "not a file: $file" >&2
  exit 1
fi

required=(
  "^## Role"
  "^## Scope"
  "^## What to check"
  "^## What NOT to flag"
  "^## Output contract"
)

missing=()
for section in "${required[@]}"; do
  if ! grep -qE "$section" "$file"; then
    missing+=("$section")
  fi
done

if [ "${#missing[@]}" -gt 0 ]; then
  echo "playbook $file is missing required sections:" >&2
  for s in "${missing[@]}"; do
    echo "  - ${s#^## }" >&2
  done
  exit 1
fi

exit 0
