---
name: merge-main-and-verify
description: Use when merging `origin/main` into the current branch before verification.
---

# Merge Main And Verify

## Overview

Merge `origin/main`, then verify against the effective branch-only diff.
Do not let a stale pushed tip classify files inherited from main as branch work.

`origin/main` is the stable baseline. Treat upstream behavior as given, not as
something to question or repair from the feature branch unless the user
explicitly asks for that work.

## When Not to Use

- The current branch belongs to a local or GitHub-managed native stack. Use
  `github-stacked-pr`; its cascading rebase is the explicit exception to the
  standalone no-rebase rule.
- The user explicitly asked for `git rebase`.
- The working tree is dirty and those changes should not be mixed into the merge.

## Workflow

### 1. Preflight

```bash
git status --short
git branch --show-current
git fetch origin
if gh stack view --json >/dev/null 2>&1; then
  echo "Refusing to merge origin/main into a native stack layer. Use github-stacked-pr." >&2
  exit 1
fi
```

`gh stack view --json` can refresh shared local stack metadata. A successful
result means this workflow must stop before the merge even if GitHub's remote
stack state later needs repair. Do not bypass the refusal by deleting local
metadata.

Rules:

- Work from the repo root.
- If the tree is dirty, do not merge blindly. Commit or stash intentionally
  first.
- Do not assume the branch already contains the latest `origin/main`.

### 2. Merge `origin/main`

```bash
git merge origin/main
```

Rules:

- Resolve conflicts without discarding user work.
- Prefer regenerating generated artifacts instead of hand-merging them. This
  includes `Cargo.lock`: on a conflict, take `origin/main`'s version and re-run
  `cargo build --locked` (or `cargo update -p <crate>` for the specific bumps
  this branch intended), rather than editing the lockfile by hand.
- Keep the merge focused on upstream sync plus fallout from the requested
  verification sequence.
- Do not make opportunistic fixes to unrelated code because `origin/main` or a
  newly merged upstream change looks suspicious.

### 3. Run the verification sequence

After the merge succeeds, run these in order:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo build --workspace --all-targets --locked
cargo test --workspace --all-features --locked
```

The whole-workspace form is correct here even though `/verify` prefers the
narrowest command. A merge integrates upstream changes whose blast radius you
did not compute, so the incremental scoping rules do not apply to the merge
commit itself.

### 4. Handle failures

- If `Cargo.lock` changed beyond the intended dependency updates, restore it and
  redo the regeneration deliberately.
- If clippy reports warnings in files this branch never touched, they came from
  upstream. Report them; do not fix them from this branch without authorization.
- If the build fails on a stale artifact, run `cargo clean -p <crate>` for the
  affected crate before assuming the merge is wrong.
- If tests fail, fix the failing scope and rerun the same command.
- If a failure appears outside the branch's authorized scope, stop before
  editing. Report the concrete failure, explain why it appears unrelated,
  propose the smallest fix or carve-out, and ask the user for guidance.
- Report the first actionable error, not a wall of logs.

### 5. Exit criteria

Done means:

- `origin/main` is merged into the current branch with no unresolved conflicts.
- The command sequence above has finished successfully.
- The working tree contains only intentional merge or verification fallout,
  inside the authorized branch scope.
- The branch is in a clear state for the next step, usually push or CI follow-up.
