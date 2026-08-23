#!/usr/bin/env bash
# Dispatch a code-review reviewer brief to OpenAI Codex CLI (GPT-5.5)
# for a second-opinion review on a high-risk diff slice.
#
# Usage:
#   dispatch-codex-reviewer.sh <reviewer-name> <brief-file> <diff-file> [output-file]
#
# Behavior:
#   - If Codex CLI is not installed or fails, returns an empty findings
#     payload with model set to `gpt-5.5-unavailable` (or `-error`) so
#     the aggregator can record the gap and the run continues cleanly.
#   - The brief should already include the reviewer playbook, the
#     shared severity rubric, the nits guide, and the output schema.
#     The helper appends the diff slice in a fenced block and asks
#     Codex to return JSON conforming to the schema.
#
# Adopted from gstack's `/codex` skill — cross-model second opinion
# catches model-specific blind spots on high-risk reviews (credential
# handling, rate-limit correctness, public API contracts, release config).

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
  "model": "gpt-5.5-unavailable",
  "findings": [],
  "note": "${reason}"
}
JSON
}

# The reason can carry raw CLI stderr, which contains quotes, backslashes and
# newlines. Escape them so the emitted payload stays parseable JSON.
json_escape() {
  printf '%s' "$1" | tr '\n\r\t' '   ' | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g'
}

emit_error() {
  local reason
  reason=$(json_escape "$1")
  cat > "${OUT}" <<JSON
{
  "reviewer": "${REVIEWER}",
  "model": "gpt-5.5-error",
  "findings": [],
  "note": "${reason}"
}
JSON
}

# Graceful degradation: if Codex CLI is not installed, emit empty
# findings and exit 0 so the run can continue.
if ! command -v codex >/dev/null 2>&1; then
  emit_unavailable "Codex CLI not installed; cross-model second opinion skipped."
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
  echo "output-schema referenced above. Set \"model\": \"gpt-5.5\" and"
  echo "\"reviewer\": \"${REVIEWER}\". If you cannot produce findings,"
  echo "return an empty findings array. Do NOT include any text outside"
  echo "the JSON object."
} > "${PROMPT}"

# Invoke Codex non-interactively in read-only sandbox (we're reviewing,
# not modifying). --ephemeral keeps the session out of Codex's history.
# --output-last-message writes the final assistant message to a file
# (that's the JSON payload the prompt asked for). The `|| emit_error`
# guard keeps the helper from failing the parent orchestrator on any
# Codex runtime error.
#
# model_reasoning_effort is pinned rather than inherited. Codex otherwise picks
# up the calling agent's effort setting, and the OpenAI API rejects values it
# does not share, such as Claude Code's `max`:
#   Invalid value: 'max'. Supported values are: 'none', 'minimal', 'low',
#   'medium', 'high', and 'xhigh'. (400, param reasoning.effort)
# That 400 made every high-risk review silently fall back to single-model.
CODEX_EFFORT="${CR_CODEX_REASONING_EFFORT:-high}"

LAST_MSG=$(mktemp -t cross-model-last-msg-XXXXXX)
CODEX_ERR=$(mktemp -t cross-model-err-XXXXXX)
trap 'rm -f "${PROMPT}" "${LAST_MSG}" "${CODEX_ERR}"' EXIT

# stderr is captured rather than discarded, so a failure reports its real cause
# instead of blaming the install.
if codex exec \
    -m gpt-5.5 \
    -c model_reasoning_effort="${CODEX_EFFORT}" \
    --sandbox read-only \
    --ephemeral \
    --skip-git-repo-check \
    --output-last-message "${LAST_MSG}" \
    < "${PROMPT}" >/dev/null 2>"${CODEX_ERR}"; then
  if [ -s "${LAST_MSG}" ]; then
    cp "${LAST_MSG}" "${OUT}"
  else
    emit_error "Codex returned an empty response."
  fi
else
  emit_error "Codex CLI invocation failed: $(tr '\n' ' ' < "${CODEX_ERR}" | tail -c 400)"
fi

exit 0
