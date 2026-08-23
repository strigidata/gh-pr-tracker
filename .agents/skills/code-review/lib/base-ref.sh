#!/usr/bin/env bash
# Pick the effective base ref for diff-driven tooling.
# A validated $CR_BASE_REF_OVERRIDE wins when the caller owns the diff window.
# Otherwise, $BASE_REF is set to either origin/<current-branch>
# when it is a clean pushed checkpoint or origin/main.
#
# Either way, a remote-tracking ref is fetched before it is used.
# A stale ref would silently narrow the reviewed and tested diff window.
#
# Both /verify and /code-review source this file so they always
# agree on the diff window.

# Refresh a remote-tracking override so validation and diffs never read a stale ref.
# A local ref that merely contains a slash is left alone.
_fetch_base_ref_override() {
  local override remote branch

  override=$1
  case ${override} in
    */*) ;;
    *) return 0 ;;
  esac

  remote=${override%%/*}
  branch=${override#*/}
  git remote get-url "${remote}" >/dev/null 2>&1 || return 0
  git fetch --no-tags --quiet "${remote}" "${branch}" 2>/dev/null || true
}

# Select a validated caller override or the established standalone fallback.
_select_base_ref() {
  local branch main_only_file file_path

  if [ -n "${CR_BASE_REF_OVERRIDE:-}" ]; then
    _fetch_base_ref_override "${CR_BASE_REF_OVERRIDE}"
    if ! git rev-parse --verify --quiet "${CR_BASE_REF_OVERRIDE}^{commit}" >/dev/null; then
      printf "Base ref override '%s' does not exist.\n" "${CR_BASE_REF_OVERRIDE}" >&2
      return 1
    fi
    if ! git merge-base --is-ancestor "${CR_BASE_REF_OVERRIDE}" HEAD; then
      printf "Base ref override '%s' is not an ancestor of HEAD.\n" "${CR_BASE_REF_OVERRIDE}" >&2
      return 1
    fi
    BASE_REF=${CR_BASE_REF_OVERRIDE}
    return 0
  fi

  branch=$(git symbolic-ref --quiet --short HEAD || true)
  if [ -n "${branch}" ]; then
    git fetch --no-tags --quiet origin "${branch}" 2>/dev/null || true
  fi
  git fetch --no-tags --quiet origin main 2>/dev/null || true

  if [ -n "${branch}" ] \
    && git rev-parse --verify --quiet "origin/${branch}" >/dev/null \
    && git merge-base --is-ancestor "origin/${branch}" HEAD; then
    BASE_REF="origin/${branch}"
  else
    BASE_REF="origin/main"
  fi

  if [ "${BASE_REF}" != "origin/main" ] \
    && git rev-parse --verify --quiet origin/main >/dev/null \
    && git merge-base --is-ancestor origin/main HEAD; then
    main_only_file=$(
      git diff --name-only --diff-filter=ACDMRTUXB "${BASE_REF}...HEAD" |
        while IFS= read -r file_path; do
          if git diff --quiet origin/main -- "${file_path}"; then
            printf '%s\n' "${file_path}"
            break
          fi
        done
    )
    if [ -n "${main_only_file}" ]; then
      BASE_REF="origin/main"
    fi
  fi
}

if ! _select_base_ref; then
  unset -f _select_base_ref _fetch_base_ref_override
  return 1 2>/dev/null || exit 1
fi

export BASE_REF
unset -f _select_base_ref _fetch_base_ref_override
