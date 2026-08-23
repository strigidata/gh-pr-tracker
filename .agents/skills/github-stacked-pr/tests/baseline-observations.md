# Stacked PR Skill RED Baseline

These observations came from three fresh agents that had the repository guidance but did not have the `github-stacked-pr` skill. The scenarios combined deadline pressure, moved trunk state, worktree isolation, tool-version uncertainty, and history-rewriting risk.

## Deadline and moved trunk

The agent interpreted the root instruction to merge `origin/main` and never rebase as absolute. It rejected GitHub's native stack workflow, proposed merge commits from `main` into the bottom layer and from the bottom layer into the child, and would have created an ordinary dependent pull request with `gh pr create`.

The agent also had no supported immediate-parent override for `/code-review`. It chose the broader `origin/main...HEAD` window and described the GitHub base branch as the only layer-diff mechanism.

The shipped guidance must make the routing exclusive: standalone branches use `merge-main-and-verify`, while confirmed native stacks use this stack workflow and never merge `origin/main` into a layer.

## Separate worktrees

The agent correctly refused to cascade-rebase while two stack branches were checked out in separate worktrees. It proposed preserving the sibling worktree, services, and database by detaching it at a captured SHA instead of deleting it.

It also identified a repository-specific side effect: a normal `git switch --detach` invokes any `post-checkout` hook, which can mutate the sibling worktree. The shipped workflow needs a documented hook-suppressed detach command, followed by explicit worktree, SHA, and service-state verification. It must never delete a database, remove a worktree, or pass `--force` merely to reconcile stack ownership.

## Tooling and metadata

The agent again treated the root no-rebase rule as overriding the requested native-stack workflow. This confirms that a short exception in root guidance is required in addition to the dedicated skill.

The investigation also exposed that `gh stack view --json` is not filesystem-read-only. The extension refreshes pull-request state and saves shared `gh-stack` Git metadata. A preflight may use it, but the workflow must call out this local metadata mutation and must not promise a side-effect-free probe.

The agent found that an interrupted cascade can leave `gh-stack-rebase-state` containing restoration refs older than current branch heads. Blindly running `gh stack rebase --abort` can therefore rewind branch refs. The shipped recovery flow must inspect and report the saved restoration SHAs against current local and remote heads before choosing `--abort`.

Finally, local stack metadata, local refs, remote refs, live pull-request bases, and GitHub's remote stack object are distinct state layers. A surprising `needsRebase` flag is not enough evidence for mutation. The workflow must compare all relevant layers with current tools and independently verify ancestry after every cascade.

## Required rationalization counters

- “The root says never rebase” is not a reason to invent merge-based stacks after native-stack routing is confirmed.
- “The command only prints JSON” is not proof that `gh stack view --json` leaves shared Git metadata untouched.
- “Both worktrees are clean” does not make a multi-worktree cascade safe.
- “Abort restores safety” is false until the saved restoration refs are compared with current branch heads.
- “The PR base looks right” does not prove local and remote native-stack metadata agree.
- “The deadline is close” does not authorize manual force-pushes, upgrades, worktree deletion, pruning, or pull-request merges.

## GREEN pressure-test result

Fresh agents given the skill correctly selected native cascading rebases instead of merge-based stacks, preserved a single worktree without deleting its database or services, stopped on unverifiable or stale tooling, treated recovery abort as potentially destructive, selected immediate-parent review bases plus cumulative verification, and kept merge authorization separate.

Two agents still ran `gh stack push` immediately before the first `gh stack submit`. The extra push is redundant because `submit` pushes active layers itself, and it adds another non-atomic failure boundary. One agent also proposed `gh stack sync` as a generic managed-update shortcut even though it pushes before the local review and verification gates. The final skill now reserves `submit` alone for initial publication, `rebase` followed by gated `push` for normal updates, and `sync` for explicit remote reconciliation such as the state after a bottom-layer merge.

A targeted retry still confused normal update and post-merge reconciliation after the distinction appeared only in their later sections. The final skill therefore puts a state-to-command routing table beside the non-negotiable invariants.

The live worktree consolidation also proved that `wt list --json` returns an object whose numeric slots live under `.data`, not an array. Its stored `branchName` can be stale after manual branch changes. The skill now gives the exact `.data | to_entries[]` filter and treats Git as authoritative for checkout ownership while using `wt` only for slot, database, service, and port preservation.

## Live native-stack adoption and cascade

The live adoption used `gh 2.97.0` and `gh-stack 0.1.0` with `gh stack init --base main feat/accounting-policy-ui chore/github-stacked-pr-workflow`. The extension adopted both existing branches and found the existing bottom PR without requiring a new worktree.

`init` recorded the older local `main` head and did not immediately enable Git `rerere`. The first interactive `gh stack rebase` fetched and fast-forwarded the trunk, then prompted to enable `rerere`. The automation PTY paused on a terminal cursor-position query before rendering the prompt, so a buffered answer was unsafe. The workflow now requires an interactive first cascade and an explicit `git config --get rerere.enabled` check.

The bottom branch still carried source commits from three earlier layers that GitHub had squash-merged. The cascade replayed those commits and surfaced conflicts instead of automatically recognizing every duplicate. Each duplicate was verified against its merged PR before retaining current-main content and allowing the commit to become empty. The real #3767 commit then required semantic conflict handling, while current-main versions of generated route manifests were retained for later regeneration.

Each `gh stack rebase --continue` returned exit 1 after committing the resolved patch and advancing to the next conflict. The final continuation completed both the bottom rebase and the downstream workflow branch rebase in one call. The exact current `REBASE_HEAD`, unmerged index, and status were the only reliable indicators after each continuation.

After the command reported success, Git ancestry was correct but the first `gh stack view --json` returned the old bottom head while persisting the rewritten head. A second view returned the current topology. The validator now performs a refresh read followed by a persisted-state read, and its focused test reproduces that two-phase metadata behavior.

The active stack metadata lived under the owning worktree's Git path. The detached former #3767 worktree retained its older stack metadata and an older recovery file without affecting the new stack. Those files were preserved rather than deleted. The active metadata also retained the local pre-adoption `trunk.head` after refreshed views, even though each branch base and Git ancestry used the captured current `origin/main`; the workflow therefore validates remote trunk ancestry rather than the persisted trunk-head field.

The first post-cascade ancestry check passed while the rewritten #3767 tree still contained literal conflict markers in generated route manifests and reintroduced two migration-test files that were absent from both current `main` and the reviewed #3767 head. Current `main` had only one unique axios dependency commit relative to that reviewed head, so 76 old-to-new file differences proved semantic replay damage rather than legitimate trunk integration. The workflow now parses every changed route manifest during topology validation and requires a separate old-head to rewritten-tree audit against trunk-only commit paths.

After the bottom tree was corrected with a normal layer commit, refreshed `gh stack view --json` output still showed the prior stored bottom head until the next cascade. That correction also produced intentional two-way divergence from the unpublished old remote head, which the original preflight rejected. The preflight now accepts this state only with the original pre-rewrite snapshot, only while every remote stack branch still equals its captured head, and only when current stack metadata owns or ancestrally tracks the local rewrite. The trunk may remain at the captured head or advance by fast-forward because `main` can move before the unpublished stack is submitted. Rewritten trunk history still fails closed. Focused tests prove the allowed correction, the fast-forward case, and refusal after unexpected remote movement or rewritten trunk history.

The first real layer-review invocation sourced the shared base-ref helper from zsh and failed because its local variable named `path` overwrote zsh's special command-path array. Renaming it to `file_path` restored command lookup, and the executable contract now sources the helper from both Bash and zsh before accepting an override.

The final one-shot review found three mutation ambiguities in `gh-stack` v0.1.0. Stack commands auto-detect a remote even though the guards validated `origin`; a bare merge number resolves as a stack number before a PR number; and interactive sync can offer remote-stack deletion and then default-yes local pruning even without `--prune`. The workflow now validates the `origin` repository identity, pins every supported remote flag, replaces numeric stack merge with an exact PR URL and head-SHA guard, and routes sync through a non-interactive wrapper that cannot accept destructive prompts.

That review also found that names and base branches alone did not prove a managed stack was current. Managed checks now require open expected PRs and agreement between each live PR or remote-stack head and `origin/<branch>`. Post-publication validation adds exact equality with the local branch so CI monitoring is always tied to the code that passed local gates.

The live stack API for PR #3767 retained closed merged PRs #3764 through #3766 before the active open member. Remote validation now compares the exact ordered open subset and its head SHAs. This preserves legitimate stack history without permitting an unexpected open layer.

Initial submission then pushed both active branches and created PR #3850 with the correct base, but the remote stack object still omitted #3850. A second `submit` produced the same warning because v0.1.0 refuses to update a remote stack when its closed historical members are absent from local metadata. After exact branch, PR, and remote-stack verification, `gh stack link 3795 https://github.com/acme/gh-pr-tracker/pull/3850 --remote origin` appended only the new PR while preserving #3764 through #3767. Managed preflight and published validation then passed. The workflow now uses this narrow link recovery instead of importing merged layers or replacing remote history.

## Bottom-merge remote rewrite and local historical metadata

The live stack #3906 used `gh 2.97.0` and `gh-stack 0.1.0`. After PR #3885 merged, GitHub retargeted PR #3901 to `main` and rewrote its remote head from the locally reviewed `51dede31` lineage to `5dcf6b4f`. The old local layer patch and rewritten remote layer patch had the same SHA-256, `2dc9f3ee17e167c104af37cb9aac74f331fb0f215d0e5124c36d48343d7eaf6e`, but local preflight correctly rejected the two-way branch divergence.

The old head was preserved as `recovery/feat-mellow-defi-positions-pre-sync-51dede31` before aligning the active branch to the proven remote rewrite. `gh stack unstack --local` removed only stale local tracking, and `gh stack checkout https://github.com/acme/gh-pr-tracker/pull/3901` re-imported stack #3906 without changing its remote object. The imported local metadata retained merged PR #3885 before open PR #3901.

Safe sync then detected a real conflict against current `origin/main`, restored every original branch, and directed the workflow to `gh stack rebase --remote origin`. The interactive cascade skipped merged PR #3885 and rebased only #3901. Its conflict progression affected only `packages/accounting-wasm/clippy-budget.baseline`; each continuation was inspected before the cumulative warning ratchet was resolved downward.

The original validator treated the retained merged local layer as active, so it either demanded the closed branch in the expected order or incorrectly required current trunk to be its ancestor. Preflight snapshots also recorded the merged bottom as the surviving top's parent, which made post-cascade movement validation reject an unchanged head already based on current trunk. Focused preflight and validator contracts now reproduce a merged historical bottom plus a trunk-based active top. Snapshots, local checks, live-PR checks, and remote-stack checks use the exact active subset while preserving closed metadata.
