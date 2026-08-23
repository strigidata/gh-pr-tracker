#!/usr/bin/env bash

set -euo pipefail

if [[ ${1:-} == "--version" ]]; then
  printf 'gh version %s (test)\n' "${FAKE_GH_VERSION:-2.97.0}"
  exit 0
fi

if [[ ${1:-} == "stack" && ${2:-} == "--version" ]]; then
  printf 'gh stack version %s\n' "${FAKE_STACK_VERSION:-0.1.0}"
  exit 0
fi

if [[ ${1:-} == "auth" && ${2:-} == "status" ]]; then
  [[ ${FAKE_AUTH_OK:-1} == 1 ]] || exit 1
  exit 0
fi

if [[ ${1:-} == "repo" && ${2:-} == "view" ]]; then
  printf '%s\n' "${FAKE_REPOSITORY:-acme/gh-pr-tracker}"
  exit 0
fi

if [[ ${1:-} == "stack" && ${2:-} == "view" ]]; then
  stack_json=${FAKE_STACK_JSON:?FAKE_STACK_JSON is required}
  if [[ -n ${FAKE_STACK_JSON_FIRST:-} && -n ${FAKE_STACK_VIEW_COUNTER_FILE:-} ]]; then
    view_count=$(cat "${FAKE_STACK_VIEW_COUNTER_FILE}" 2>/dev/null || printf '0')
    if [[ ${view_count} == 0 ]]; then
      stack_json=${FAKE_STACK_JSON_FIRST}
    fi
    printf '%s\n' "$((view_count + 1))" >"${FAKE_STACK_VIEW_COUNTER_FILE}"
  fi
  printf '%s\n' "${stack_json}"
  exit 0
fi

if [[ ${1:-} == "stack" && ${2:-} == "sync" ]]; then
  if [[ -n ${FAKE_GH_INVOCATION_FILE:-} ]]; then
    printf '%s\n' "$*" >"${FAKE_GH_INVOCATION_FILE}"
  fi
  exit "${FAKE_SYNC_EXIT_CODE:-0}"
fi

if [[ ${1:-} == "pr" && ${2:-} == "view" ]]; then
  pr_number=${3:?pull request number is required}
  response_name="FAKE_PR_${pr_number}_JSON"
  printf '%s\n' "${!response_name:?missing fake pull request response}"
  exit 0
fi

if [[ ${1:-} == "api" ]]; then
  case ${2:-} in
    repos/cli/cli/releases/latest)
      [[ ${FAKE_CLI_LOOKUP_OK:-1} == 1 ]] || exit 1
      printf '%s\n' "${FAKE_CLI_LATEST:-v2.97.0}"
      ;;
    repos/github/gh-stack/releases/latest)
      [[ ${FAKE_STACK_LOOKUP_OK:-1} == 1 ]] || exit 1
      printf '%s\n' "${FAKE_STACK_LATEST:-v0.1.0}"
      ;;
    repos/*/stacks\?pull_request=*)
      printf '%s\n' "${FAKE_REMOTE_STACK_JSON:-[]}"
      ;;
    *)
      printf 'unexpected fake gh api endpoint: %s\n' "${2:-}" >&2
      exit 2
      ;;
  esac
  exit 0
fi

printf 'unexpected fake gh invocation: %s\n' "$*" >&2
exit 2
