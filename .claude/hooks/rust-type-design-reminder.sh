#!/usr/bin/env bash
# Reminds the agent to load the Rust type-design rules the first time it edits
# a Rust file in a session.
# Stays silent on every later edit, so the reminder costs one line per session.
# Skills are model-invoked and therefore optional; this hook is harness-executed
# and is not.
set -uo pipefail

# This hook is advisory, and a PreToolUse hook that exits non-zero can block the
# tool call. Degrade to a silent no-op rather than breaking every Edit and Write
# on a machine without jq.
command -v jq >/dev/null 2>&1 || exit 0

payload=$(cat)
file=$(printf '%s' "$payload" | jq -r '.tool_input.file_path // empty' 2>/dev/null || true)
session=$(printf '%s' "$payload" | jq -r '.session_id // "nosession"' 2>/dev/null || true)

case "$file" in
*.rs)
  skill=rust-best-practices
  ;;
*)
  exit 0
  ;;
esac

sentinel="${TMPDIR:-/tmp}/claude-type-design-${session//[^A-Za-z0-9_-]/}-${skill}"
[ -e "$sentinel" ] && exit 0
: >"$sentinel"

jq -n --arg skill "$skill" '{
  hookSpecificOutput: {
    hookEventName: "PreToolUse",
    additionalContext: "Do not forget our strict type design rules: skill `\($skill)`, Illegal States category."
  }
}'
