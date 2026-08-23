#!/usr/bin/env bash
# Dispatch a code-review reviewer brief to Claude Code CLI
# (Sonnet 4.6) for a second-opinion review on a high-risk diff slice.
# This is the counterpart to dispatch-codex-reviewer.sh — use whichever
# helper invokes the OTHER model from your current host.
#
# Usage:
#   dispatch-claude-reviewer.sh <reviewer-name> <brief-file> <diff-file> [output-file]
#
# Behavior:
#   - If `claude` CLI is not installed or fails, returns an empty findings
#     payload with model set to `claude-sonnet-4-6-unavailable` (or `-error`) so
#     the aggregator can record the gap and the run continues cleanly.
#   - The brief should already include the reviewer playbook, the
#     shared severity rubric, the nits guide, and the output schema.
#     The helper appends the diff slice in a fenced block and asks
#     Claude to return JSON conforming to the schema.
#
# Adopted from gstack's `/codex` skill — cross-model second opinion
# catches model-specific blind spots on high-risk reviews. Use this
# helper when running under Codex (or any non-Claude host) to get a
# Sonnet second opinion. Use dispatch-codex-reviewer.sh when running
# under Claude Code to get a GPT-5.5 second opinion.

set -u

REVIEWER="${1:-}"
BRIEF="${2:-}"
DIFF="${3:-}"
OUT="${4:-/dev/stdout}"

if [ -z "${REVIEWER}" ] || [ -z "${BRIEF}" ] || [ -z "${DIFF}" ]; then
  echo "usage: $0 <reviewer-name> <brief-file> <diff-file> [output-file]" >&2
  exit 1
fi

for f in "${BRIEF}" "${DIFF}"; do
  if [ ! -f "${f}" ]; then
    echo "not a file: ${f}" >&2
    exit 1
  fi
done

emit_unavailable() {
  local reason="$1"
  cat > "${OUT}" <<JSON
{
  "reviewer": "${REVIEWER}",
  "model": "claude-sonnet-4-6-unavailable",
  "findings": [],
  "note": "${reason}"
}
JSON
}

emit_error() {
  local reason="$1"
  cat > "${OUT}" <<JSON
{
  "reviewer": "${REVIEWER}",
  "model": "claude-sonnet-4-6-error",
  "findings": [],
  "note": "${reason}"
}
JSON
}

# Graceful degradation: if `claude` CLI is not installed, emit empty
# findings and exit 0 so the run can continue.
if ! command -v claude >/dev/null 2>&1; then
  emit_unavailable "Claude Code CLI not installed; cross-model second opinion skipped."
  exit 0
fi

# Compose the prompt: brief + diff slice + JSON-output instruction.
PROMPT=$(mktemp -t cross-model-prompt-XXXXXX)
trap 'rm -f "${PROMPT}"' EXIT

{
  cat "${BRIEF}"
  echo ""
  echo "## Diff slice to review"
  echo ""
  echo '```diff'
  cat "${DIFF}"
  echo '```'
  echo ""
  echo "## Instruction"
  echo ""
  echo "Return findings as a single JSON object conforming to the"
  echo "output-schema referenced above. Set \"model\": \"claude-sonnet-4-6\" and"
  echo "\"reviewer\": \"${REVIEWER}\". If you cannot produce findings,"
  echo "return an empty findings array. Do NOT include any text outside"
  echo "the JSON object."
} > "${PROMPT}"

# Invoke Claude Code non-interactively via the `-p` (print) flag, which
# runs a single prompt and prints the assistant's response. Use
# claude-sonnet-4-6 explicitly so the second opinion is a known model regardless
# of the caller's default. The `|| emit_error` guard keeps the helper
# from failing the parent orchestrator on any Claude runtime error.
if ! claude -p --model claude-sonnet-4-6 < "${PROMPT}" > "${OUT}" 2>/dev/null; then
  emit_error "Claude Code CLI invocation failed; check the local Claude install."
fi

exit 0
