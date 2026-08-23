---
name: github-stacked-pr
description: Use when creating, adopting, updating, synchronizing, reviewing, verifying, submitting, or merging GitHub native stacked pull requests, including when a PR should be based on another feature PR, GitHub offers to rebase a stack, a merged bottom PR must be reconciled, or multiple stack layers are checked out in separate worktrees. Wraps the official github/gh-stack extension with worktree, version, ancestry, review, CI, authorization, and recovery gates.
---

# GitHub Native Stacked Pull Requests

Use GitHub's `github/gh-stack` extension for stack mechanics. Use the bundled
scripts for safety and verification.

```text
Standalone branch: merge-main-and-verify
Confirmed or newly requested native stack: github-stacked-pr
```

Never merge `origin/main` into a native stack layer. Cascading rebases through
`gh stack` are the explicit exception to the repository's standalone no-rebase
rule. Do not turn an ordinary dependent pull request into a native stack unless
the user requested or approved that change.

## Authorization boundaries

Treat a request to create, update, synchronize, or submit named stack layers as
authorization for the corresponding `gh stack init`, `rebase`, `push`,
`submit`, and pull-request base updates after the gates below pass.

Get separate explicit authorization before:

- creating a worktree when development is not already in one;
- installing or upgrading `gh` or `github/gh-stack`;
- removing a worktree or deleting its database or services;
- pruning or deleting branches;
- aborting recovery when saved restoration refs could discard newer commits;
- merging any pull request, including the bottom layer.

Never infer merge authorization from authorization to build or submit a stack.

## Non-negotiable invariants

- Operate the whole stack from one git worktree.
- Reuse the existing feature worktree. Never create one worktree per layer.
- Require current stable `gh` and `github/gh-stack` versions before mutation.
- Pin every stack fetch or push to `--remote origin` and validate that `gh`
  resolves the same GitHub repository as that remote.
- Let `gh stack` own cascading rebases and rewritten-branch pushes.
- Never run manual `git rebase`, `git pull`, `git push --force`, or
  `git push --force-with-lease` on stack layers.
- Capture the remote trunk and every layer head before a cascade.
- Independently validate ancestry after a successful command.
- Review each layer against its immediate parent and verify the top layer
  cumulatively against `origin/main`.
- Require green CI and resolved review threads on every current PR head.

Choose the mutation from current state before running commands:

| State                                                | Required path                                                                                                  |
| ---------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| Existing branches, at least one missing PR           | `init`, cascade `rebase`, local gates, then `submit --auto`                                                    |
| Managed stack with a normal layer edit               | Managed preflight, cascade `rebase`, local gates, then `push`                                                  |
| Bottom PR merged or remote stack composition changed | Local preflight, explicit reconciliation with safe non-interactive `sync`, then post-sync validation and gates |
| Ready to merge                                       | `merge` only with separate explicit authorization                                                              |

Never use `sync` for a normal layer edit. It pushes during the command before
the local review and verification gates can run.

## Understand local and remote state

Keep these state layers distinct:

1. Local branch refs.
2. Remote-tracking branch refs.
3. Current-worktree `gh-stack` metadata.
4. Live pull-request heads and bases.
5. GitHub's remote stack object.
6. Optional `gh-stack-rebase-state` or `gh-stack-modify-state` recovery data.

`gh stack view --json` is machine-readable but not filesystem-read-only. It
refreshes pull-request state and saves shared Git metadata. Run it only after
checking the working tree, sibling worktrees, and recovery state. Do not use its
`needsRebase` value as the sole reason to mutate branches.

The first view after a successful cascade can return the pre-refresh heads while
persisting the new heads for the next read. The bundled preflight and validator
therefore call it once to refresh and a second time to verify the persisted
view. Do not replace post-cascade validation with one raw `view --json` call.

The metadata and recovery files are stored under the current worktree Git path.
A preserved detached sibling can retain an older `gh-stack` file or recovery
file without changing the active stack. Do not delete or reconcile those files
manually. Run every stack command from the sole owning worktree and treat its
Git path as authoritative for that operation.

GitHub's remote stack object can retain closed, already-merged historical PRs
before the active local layers. The guards compare the exact ordered open
subset and its head SHAs; they do not reject valid history or let an unexpected
open layer hide among closed members. When adding a new top layer, `submit` can
refuse to update that remote object because the closed members are absent from
local metadata even after it successfully pushes the branches and creates the
new PR.

Imported local metadata can retain the same merged historical layers after a
bottom PR lands. Topology and publication guards compare the exact active local
subset while preserving those closed records. Preflight snapshots serialize
only that active subset so the surviving PR records the trunk as its parent.
Supply only the active branches to `validate-stack.sh`; a merged historical
branch is not an ancestry parent of the surviving PR after GitHub retargets that
PR to the trunk.

## Prepare one stack worktree

Inspect ownership first:

```bash
git status --short --branch
git worktree list --porcelain
```

Use Git for current branch and checkout ownership. Every layer of the stack is
operated from a single worktree; never create one worktree per layer.

If no feature worktree exists, stop and ask before creating one. See
`superpowers:using-git-worktrees` for the creation workflow.

If another clean worktree checks out a stack layer, preserve its checkout and
branch ref by detaching it at a captured SHA, while suppressing any repository
`post-checkout` hook:

```bash
sibling_sha=$(git -C "$sibling_path" rev-parse HEAD)
git -C "$sibling_path" status --short --branch
git -C "$sibling_path" -c core.hooksPath=/dev/null switch --detach "$sibling_sha"
git worktree list --porcelain
git -C "$sibling_path" rev-parse HEAD
```

Stop if the sibling is dirty. Do not stash it automatically. Detaching is
reversible and does not delete the worktree. Remove a worktree only when the
user separately authorizes removal.

## Check tooling and preflight

Run the bundled preflight before any stack mutation. It confirms installed and
latest stable versions, authentication, Git state, branch divergence,
worktree ownership, the fixed `origin` repository identity, and the requested
metadata level.

```bash
skill_dir=.agents/skills/github-stacked-pr
snapshot=$(mktemp -t stack-before-XXXXXX.json)
```

Use exactly one mode:

- `adopt`: existing branches are not locally tracked as a stack yet.
- `local`: local stack metadata exists, but not every layer has a PR yet.
- `managed`: local metadata, live PR chaining, and GitHub's remote stack object
  must all agree.

```bash
bash "$skill_dir/scripts/preflight.sh" adopt --snapshot "$snapshot" bottom top
bash "$skill_dir/scripts/preflight.sh" local --snapshot "$snapshot"
bash "$skill_dir/scripts/preflight.sh" managed --snapshot "$snapshot"
```

After a validated cascade and before its first publication, rewritten branches
normally diverge from their old remote heads. Snapshots record both the local
layer head and its published remote head so an unpublished layer edit does not
erase the lease evidence. If another corrective cascade is required, prove that
every remote stack branch still equals the original published head and that the
trunk has only advanced by fast-forward:

```bash
bash "$skill_dir/scripts/preflight.sh" local \
  --rewrite-snapshot "$original_snapshot" \
  --snapshot "$fresh_snapshot"
```

The exception is snapshot-backed. It is allowed only when the current remote
head still equals the captured `remoteHead` (falling back to `head` for older
snapshots), the remote trunk is the captured trunk or a descendant of it, and
current stack metadata owns the local rewrite.
The preflight checks live remote existence and removes a stale tracking ref when
a branch was deleted; a branch with captured `remoteHead` lease evidence must
still exist. A transport or authentication failure while querying a branch is
not absence and stops preflight. Any remote stack-branch movement or rewritten
trunk history fails closed. This fast-forward allowance is necessary when
`main` advances before an unpublished stack can be submitted. Never add a
general divergence bypass.

Preflight is intentionally fail-closed. If the latest release cannot be
confirmed or an installed version is stale, explain the required upgrade and
ask before running it. Detect the installation method instead of guessing.

An adoption preflight accepts non-linear existing layers when each layer shares
a merge base with its declared parent. `gh stack init` records those merge
bases; the first cascade makes the declared order linear. Post-cascade
validation is strict.

## Run every cascade with process-local hook suppression

Define this helper once in the shell before any initial, corrective, or
continuation cascade:

```bash
gh_stack_rebase() {
  local config_count="${GIT_CONFIG_COUNT:-0}"
  case "$config_count" in
    '' | *[!0-9]*)
      printf 'Invalid GIT_CONFIG_COUNT: %s\n' "$config_count" >&2
      return 2
      ;;
  esac
  env \
    "GIT_CONFIG_COUNT=$((config_count + 1))" \
    "GIT_CONFIG_KEY_${config_count}=core.hooksPath" \
    "GIT_CONFIG_VALUE_${config_count}=/dev/null" \
    gh stack rebase --remote origin "$@"
}
```

The environment override is inherited by the Git subprocesses created by
`gh stack`, preserves any existing process-local Git configuration entries, and
disappears when the command exits. It never writes repository, worktree, global,
or included Git configuration. The helper returns the exact cascade status.

## Create or adopt a stack

1. Run `adopt` preflight with ordered branches from bottom to top.
2. Initialize the local stack from the existing worktree.
3. Re-run `local` preflight and capture a fresh snapshot.
4. Cascade-rebase the declared order.
5. Validate local topology before the first submission.

```bash
gh stack init --base main bottom top
bash "$skill_dir/scripts/preflight.sh" local --snapshot "$snapshot"
trunk_sha=$(jq -r .trunkHead "$snapshot")
gh_stack_rebase
bash "$skill_dir/scripts/validate-stack.sh" \
  --local --trunk "$trunk_sha" --before "$snapshot" bottom top
bash "$skill_dir/scripts/diff-summary.sh" --trunk origin/main bottom top
```

`gh stack init` accepts existing branches and switches to the top layer. Always
pass branch names to avoid interactive branch selection.

In practice, `init` can leave `rerere` disabled and defer the enablement prompt
until the first interactive `gh stack rebase`. Run that first cascade in an
interactive terminal, answer the prompt deliberately, and verify the result:

```bash
git config --get rerere.enabled
```

An automation PTY can pause while querying terminal cursor position before it
renders the prompt. Do not buffer an answer blindly or classify the command as
hung without inspecting the terminal exchange.

`init` can also record the current local `main` head even when `origin/main` is
newer. The cascade fetches and resolves the remote trunk before replaying the
bottom layer. The preflight snapshot and post-cascade ancestry validator remain
authoritative; the initial `needsRebase` value does not.

The persisted `trunk.head` can remain at that older local SHA even after the
cascade and refreshed views. Validate the captured `origin/main` SHA and each
branch base instead of treating the stored trunk head as current.

## Update an existing stack

1. Check out the layer that owns the change with `gh stack checkout <branch>`.
2. Make and commit the layer-scoped change.
3. Check out the top layer.
4. Run `managed` preflight and capture the snapshot.
5. Run the full cascading rebase.
6. Validate the captured trunk, every parent-child edge, local metadata, live
   pull-request bases, and remote stack order.
7. Review layers and verify the cumulative top before pushing.

```bash
bash "$skill_dir/scripts/preflight.sh" managed --snapshot "$snapshot"
trunk_sha=$(jq -r .trunkHead "$snapshot")
gh_stack_rebase
bash "$skill_dir/scripts/validate-stack.sh" \
  --trunk "$trunk_sha" --before "$snapshot" bottom top
```

A normal Git commit on a checked-out layer does not immediately rewrite the
stored head shown by `gh stack view --json`. Before the cascade, preflight allows
that stored head only when it is an ancestor of the real branch ref. The cascade
updates stack metadata; post-cascade validation then requires exact head equality.

A zero exit from `gh stack rebase` is necessary but insufficient. The validator
detects a child that did not move after its parent changed and refuses broken
ancestry, stale metadata, or malformed changed route manifests.

## Audit rewritten trees

Ancestry can be correct while replay changes differ from the previously reviewed
branch tree. This is especially likely when older stack layers were
squash-merged and the surviving branch contains merge commits that the cascade
does not replay.

For an adopted branch, compare the captured pre-cascade head with the rewritten
ref. List trunk-only commits and their paths separately:

```bash
old_bottom=$(jq -r '.branches[] | select(.name == "bottom") | .head' "$snapshot")
git log --oneline "$old_bottom..$trunk_sha"
git log --format= --name-only "$old_bottom..$trunk_sha" | sort -u
git diff --name-status "$old_bottom" bottom
```

Every old-to-new tree difference must come from a real trunk-only change or an
explicit semantic conflict resolution. If the rewritten tree reintroduces files
absent from both the reviewed head and current trunk, removes reviewed files, or
contains conflict markers, fix the owning layer before review or push. Do not
accept topology validation as proof of semantic equivalence.

The validator scans every changed file in every layer for unresolved conflict
markers and parses every changed `*.json` file with `jq`. This catches a layer
committed mid-conflict, which ancestry validation alone will not notice. It does
not prove semantic correctness — regenerate any generated artifact from the
final rebased tree during verification.

## Review and verify

Print both diff windows:

```bash
bash "$skill_dir/scripts/diff-summary.sh" --trunk origin/main bottom top
```

For every changed layer, check out the layer and set the validated immediate
parent before invoking `/code-review`:

```bash
gh stack checkout top
export CR_BASE_REF_OVERRIDE=bottom
# Invoke /code-review.
unset CR_BASE_REF_OVERRIDE
```

From the top layer, verify the complete stack against production main:

```bash
gh stack top
export CR_BASE_REF_OVERRIDE=origin/main
# Invoke /verify and report its full step-by-step result table.
unset CR_BASE_REF_OVERRIDE
```

The shared helper rejects a missing or non-ancestor override. Do not set an
override merely to shrink a diff. The validated stack topology owns it.
The helper is sourced from both Bash and zsh agent shells. Its regression test
runs both shells because `path` is a special zsh array that can silently remove
`git` from command lookup when reused as a local variable.

## Push and submit

### Do not push an upper layer while a lower one is red

Every push fans CI out across the whole stack: each layer runs its own build,
test, and preview jobs. Pushing layer N while layer N-1 is failing spends that
entire fan-out on a base that is known broken, and the upper layers usually fail
for the lower layer's reason anyway, burying the real signal.

Before pushing any layer above the bottom, confirm every layer below it is green:

```bash
gh pr checks <lower-pr> | awk -F'\t' '{print $2}' | sort | uniq -c
```

Fix the lowest red layer, push only that layer, and let it go green before moving
up. The cascade keeps the upper layers correct locally in the meantime; they lose
nothing by waiting. CI here is expensive and the runners are shared.

The same reasoning applies to the red layer itself: reproduce and prove the fix
locally rather than pushing successive attempts and reading the job log. `/verify`
carries a Docker gate for exactly this case, because a new package or crate fails
only in the image build.

When every layer already has a pull request, push rewritten managed layers only
through the extension:

```bash
gh stack push --remote origin
```

`gh stack push` uses an explicit lease per branch, but the multi-branch update
is not atomic. Some branches can move before a later lease fails. Re-run
preflight and topology validation before retrying.

When any layer still needs a pull request, create it and the GitHub stack object
with non-interactive titles:

```bash
gh stack submit --auto --remote origin
```

Do not run `gh stack push` immediately before an initial `submit`. `submit`
already pushes every active layer, so the extra command only creates a second
partial-mutation boundary without adding safety.

`submit` pushes branches sequentially and can also partially complete. Re-run
the same command after fixing a rejected layer. Pass `--open` only when the user
explicitly asks to mark drafts ready for review.

### Normalize every PR `submit` creates, before anything else

`submit` accepts no title or body. It derives the title from the **branch name**
and leaves the body as the **unfilled template**. Both are wrong on arrival:
`refactor/accounting-domain-crate` becomes the title
`refactor/accounting domain crate`, which fails the `Validate PR Title` check
because it is not `type: lowercase description`.

This is not optional cleanup and it is not "later" — the PR is wrong the moment
it exists, and a reviewer or a CI check sees it before you do. Immediately after
any `submit` that reports `Created PR #N`:

```bash
gh pr edit <N> --title 'type: lowercase description'
gh pr edit <N> --body-file <(...)   # sections from .github/pull_request_template.md, filled
```

Do this for each created PR before running gates, watching CI, or moving to the
next layer. `gh pr create --body` is not an escape from the template either; the
sections must be reproduced and answered, including Security Impact in prose.

`submit` invokes normal Git pushes, so repository pre-push hooks run for each
layer. Never bypass a rejected hook with `--no-verify` or an equivalent
escape hatch. Compare every remote branch and live PR head with the
preflight snapshot before retrying because an earlier layer may already have
moved even when the command exits with a hook failure.

After submission, run `managed` preflight and published validation again:

```bash
bash "$skill_dir/scripts/validate-stack.sh" \
  --published --trunk "$trunk_sha" bottom top
```

Published validation requires every expected PR to be open and requires the
local branch, `origin` tracking ref, live PR head, and remote-stack head to have
the same SHA. Confirm that the top PR diff excludes all parent-only changes.
Then watch CI to completion for every rewritten PR head:
`gh pr checks <pr> --watch`.

If the PR chain and heads are correct but managed preflight proves that an
existing remote stack with closed historical members omits the new top PR, do
not replace the remote stack or import merged layers merely to satisfy local
metadata. Resolve the existing stack number from the GitHub stack API, run a
fresh local preflight, and append only the exact new PR URL:

```bash
gh stack link <stack-number> https://github.com/OWNER/REPO/pull/<new-pr> \
  --remote origin
```

`link` is the supported external-tracking path and preserves every existing
remote member. Re-run managed preflight and published validation immediately.
Use this recovery only when the existing remote members, PR bases, remote
heads, and desired append order were all verified first.

## Synchronize after a bottom merge

GitHub may retarget the next PR without rewriting its head. Recheck live heads,
bases, and stack metadata instead of assuming the cascade completed.

If the remaining stack is incomplete or stale, run from its sole worktree:

```bash
sync_log=$(mktemp -t stack-sync-XXXXXX.log)
bash "$skill_dir/scripts/preflight.sh" local --snapshot "$snapshot"
bash "$skill_dir/scripts/safe-sync.sh" </dev/null >"$sync_log" 2>&1
cat "$sync_log"
```

The wrapper refuses terminal input or output, so the invocation detaches stdin
and captures the transcript in `$sync_log`. Read the result from that file.

Use `sync` only for explicit remote-stack reconciliation, especially after a
bottom-layer merge. Do not substitute it for the normal managed update sequence
of `rebase`, local review and verification, then `push`: `sync` pushes as part
of the command before those local gates can run.

The wrapper invokes
`gh stack sync --remote origin` without `--prune`. This matters because
`gh-stack` v0.1.0 can otherwise offer to delete the remote stack during
divergence handling and prompts to prune merged branches with a default of yes.
Any divergence in the non-interactive path cancels without pushing, so report
it and request direction instead of rerunning interactively.

GitHub can rewrite the surviving remote branch while merging the bottom PR,
leaving a clean but two-way-diverged local branch. Before accepting the remote
rewrite, compare the old layer patch with the rewritten remote layer patch and
account for every difference. After the user selects the proven remote rewrite
as the source of truth, preserve the complete old local head under a recovery
branch before aligning the active branch. If the local stack metadata still
records the old head, use the extension's local-only re-adoption path:

```bash
gh stack unstack --local
gh stack checkout https://github.com/OWNER/REPO/pull/<active-pr>
```

`unstack --local` removes only stale local stack tracking. It does not delete
branches or modify GitHub's stack. Re-run local preflight before safe sync.

`sync` fetches, reconciles remote stack composition, updates trunk,
cascade-rebases, atomically pushes active branches, and synchronizes PR state.
It never opens missing PRs. Never bypass the wrapper or pass `--prune` without
separate branch-deletion authorization.

After sync, repeat strict published validation, layer review, cumulative
verification, and current-head CI monitoring.

## Suppress the `post-checkout` hook around a cascade

`gh stack rebase` checks out each layer in turn. Any repository
`post-checkout` hook fires on every one of those checkouts. A hook that
regenerates artifacts produces a file belonging to whichever layer was checked
out a moment ago, so the tree is dirty by the time the next layer is replayed
and the cascade stops with:

```text
✗ could not start rebase of <branch> onto origin/main: failed to run git:
  error: cannot rebase: You have unstaged changes.
```

The tree is clean before and after, which makes this look like a phantom. It is
the hook, and it recurs on every cascade.

Disable hooks for the cascade process only with the `gh_stack_rebase` helper
defined above:

```bash
gh_stack_rebase
```

Do not replace the process-local override with `git config`. There is nothing to
restore after success, failure, or interruption because the parent shell and all
persisted Git configuration remain unchanged.

This suppression covers the cascade's _internal_ checkouts. It is the same
technique the sibling-detachment step above uses, and it is equally narrow: it
must never extend to `push`, `submit`, or `sync`, whose pre-push hooks are real
gates. Never reach for `--no-verify` or an equivalent hook-disabling
environment variable instead.

If the regenerated artifact turns out to be genuinely stale rather than
cross-layer noise — the committed file disagrees with its generator on the layer
that owns it — commit the regenerated file to that layer. That is a real defect
the hook surfaced, not an obstacle to work around.

## Resolve and recover from conflicts

On the first conflict:

```bash
git status --short --branch
git ls-files -u
git rev-parse --git-path gh-stack-rebase-state
```

Resolve only the logical changes owned by the current layer, stage them, and
continue:

```bash
git add <resolved-files>
gh_stack_rebase --continue
git status --short --branch
git ls-files -u
```

Inspect status after every continuation. One call can commit the resolved
change, advance into another commit or layer, and then return exit 1 with only a
generic conflict message. Git status, `REBASE_HEAD`, and the current patch are
authoritative. Do not retry the previous resolution or assume the same commit is
still active.

When an older stack layer was already squash-merged, compare the active patch
with the merged PR before resolving it. Only for a proven duplicate should the
current parent version win so the replayed commit becomes empty. Never apply a
blanket ours-or-theirs choice to a commit that still owns unmerged behavior.

Keep the current-trunk version of any conflicted **generated** file during the
cascade — `Cargo.lock` is the common one. Regenerate it after the complete stack
is on the captured trunk (`cargo build --locked`, or `cargo update -p <crate>`
for the bumps this branch actually intended), then review the generated diff. A
later regeneration-only commit does not make stale conflict output safe.

One `gh stack rebase --continue` call resumes the complete cascade. It can finish
the bottom branch and automatically rebase every downstream branch before it
returns success. Validate the resulting refs before running any new mutation.

Before `gh stack rebase --abort`, inspect the recovery JSON and compare every
saved restoration SHA with current local and remote heads:

```bash
recovery_path=$(git rev-parse --git-path gh-stack-rebase-state)
jq . "$recovery_path"
git show-ref --heads
git ls-remote --heads origin
```

Abort only when restoration cannot discard commits created after the saved
state. If uncertain, stop and report the exact refs. Never delete recovery
metadata manually to make preflight pass.

## Merge

Merge bottom-up and one PR at a time only after explicit authorization:

```bash
stack_json=$(gh stack view --json)
bottom_pr_url=$(jq -er '.branches[0].pr.url' <<<"$stack_json")
bottom_head=$(jq -er '.branches[0].head' <<<"$stack_json")
gh pr merge "$bottom_pr_url" --squash --match-head-commit "$bottom_head"
```

Run managed preflight and `validate-stack.sh --published` immediately before
this command. The PR URL identifies the exact repository and PR, while
`--match-head-commit` prevents merging a head that moved after validation.
Never pass a bare number to `gh stack merge`: v0.1.0 resolves it as a stack
number before trying it as a PR number, so a collision can merge a different
stack or more layers than authorized.

Recheck GitHub state, synchronize the remaining stack when required, and obtain
fresh authorization for each next merge. A green child PR does not prove its
parent or the cumulative stack is green.

## Record field learnings

Treat every real stack operation as a workflow test. Before declaring the
workflow branch complete:

1. Record the exact installed versions and command used.
2. Record unexpected output, partial mutation, worktree behavior, metadata
   refresh, conflict progression, and GitHub UI retargeting.
3. Add or update a focused executable test for deterministic behavior.
4. Update this skill and the concise root routing rule when agent behavior was
   ambiguous.
5. Re-run the pressure scenarios and focused tests.

Do not leave session learnings only in chat or a plan document.

## Rationalization counters

| Shortcut                                                         | Required response                                                                                                                                                                 |
| ---------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| “Root says never rebase.”                                        | Native stacks are the named exception; use `gh stack rebase`, never merge main into a layer.                                                                                      |
| “Both worktrees are clean.”                                      | Clean siblings still block a safe cascade; consolidate ownership first.                                                                                                           |
| “`view --json` only reads.”                                      | It can refresh and save shared metadata; treat it as a local mutation.                                                                                                            |
| “The successful rebase means one view is current.”               | The first post-cascade view can return stale heads while persisting fresh metadata; use the two-read validator.                                                                   |
| “`init` enabled rerere already.”                                 | The first interactive rebase can still prompt; answer deliberately and verify Git config.                                                                                         |
| “Continue returned 1, so it did nothing.”                        | It can commit and advance before the next conflict; inspect Git state and the current patch.                                                                                      |
| “Ancestry validation passed.”                                    | Audit old and rewritten trees; a cascade can replay files that the reviewed final tree had removed.                                                                               |
| “Abort always restores safety.”                                  | Inspect saved restoration refs before allowing `--abort`.                                                                                                                         |
| “The PR base looks right.”                                       | Compare local metadata, live PRs, remote stack order, and ancestry.                                                                                                               |
| “The push command is atomic.”                                    | `push` and `submit` can partially complete; validate before retrying.                                                                                                             |
| “The default remote is obvious.”                                 | Pin `--remote origin`; preflight must prove `gh` resolves the same repository.                                                                                                    |
| “The tree is clean, so the cascade failure is a `gh-stack` bug.” | A `post-checkout` hook can regenerate artifacts during internal checkouts; use `gh_stack_rebase` so persisted Git configuration stays untouched.                        |
| “Divergence is expected after rebase, so ignore it.”             | Use the pre-rewrite snapshot; remote stack heads must still match exactly and the trunk may only advance by fast-forward.                                                         |
| “Push before submit is safer.”                                   | Initial `submit` already pushes; do not add a redundant partial-mutation boundary.                                                                                                |
| “Submit said synced, so the remote stack is complete.”           | Managed validation owns the answer. If preserved closed history blocks an append, verify the remote object and use `gh stack link <stack-number> <exact-pr-url> --remote origin`. |
| “Sync is the easiest update command.”                            | Reserve `sync` for explicit remote reconciliation; normal updates gate a `rebase` before `push`.                                                                                  |
| “Plain sync is safe without `--prune`.”                          | v0.1.0 can still prompt for deletion and default-yes pruning; use only the non-interactive safe-sync wrapper.                                                                     |
| “The number is clearly a PR.”                                    | `gh stack merge` tries stack numbers first; merge one authorized PR by exact URL and validated head SHA.                                                                          |
| “CI is green on the child.”                                      | Require current-head CI and reviews for every layer plus cumulative verification.                                                                                                 |
| “The deadline authorizes cleanup.”                               | Never infer upgrades, deletion, pruning, or merge permission.                                                                                                                     |
