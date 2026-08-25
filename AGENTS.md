# AGENTS.md

Project instructions for AI coding agents (Claude Code, Codex, OpenCode, Cursor, etc.).
`CLAUDE.md` symlinks here.

## Project overview

`gh-pr-tracker` watches GitHub pull requests and interrupts a developer at the
moment they are blocking their team — or being blocked by their own finished
work.

Current state: **shipped and in use.** `prtrack board` renders the board,
`--watch` polls and notifies on transitions, and releases are cut from CI for
macOS and Linux. See [`README.md`](README.md) for what it does and
[`docs/plans/`](docs/plans/) for why it does it that way — the plan documents
are a **historical record of decisions**, not instructions; where they disagree
with this file or the code, they are the ones that are wrong.

```
crates/prtrack-core     no IO, no clock, no terminal. Newtypes, the
                        thirteen-rung ladder, the board's sections and
                        ordering, and the --watch transition rules.
crates/prtrack-github   transport. Wire types, pagination, retry, policy
                        resolution, and the conversion into domain facts.
crates/prtrack-cli      the shell. The `prtrack` binary plus a library half
                        holding the renderer, colour, exit codes, state file,
                        notifications and the update check.
scripts/                two surviving tools: check-states.sh, which fails when
                        the repository produces a check state the crate does
                        not know, and pr-latency.sh.
```

The binary is named `prtrack`, not `prtrack-cli`. The shell prototype that
served as this port's oracle has been deleted; it is recoverable from history
if a second implementation is ever needed to diff against.

The split is load-bearing:

- **Library crates** — fetch, model, and reduce PR state, then decide which
  changes deserve an interruption. They know nothing about terminals. The macOS
  SwiftUI app consumes them over a UniFFI boundary and a hosted backend will
  consume them unchanged, so their public API is a real contract with callers
  that do not exist yet and cannot be refactored alongside you.
- **Shells** — a CLI/TUI binary, and later a native macOS menubar app. Argument
  parsing, config resolution, rendering, notifications, process lifecycle.

Anything that assumes a terminal, an ANSI code, or a TTY belongs in a shell.
If a library crate needs it, the abstraction is wrong. `prtrack-core`
specifically does no IO at all.

## Core tenets

1. **The interruption is the product, not the table.** No team is unblocked
   because a dashboard was accurate. They are unblocked because someone was
   poked at the right moment and acted. The table is a debugging view of the
   state that drives the nudge.
2. **Correctness of that state.** A wrong cell is worse than a missing one, and
   a stale cell that looks fresh is worse than both. Failure must always be
   distinguishable from an empty result — an empty table that means "GitHub is
   down" is a lie to the user.
3. **A false nudge costs more than a missed one.** Notification fatigue gets the
   tool uninstalled, and an uninstalled tool unblocks nobody. When in doubt, stay
   quiet.
4. **Illegal states are unrepresentable.** Prefer a type that cannot express a
   wrong value over a runtime check that rejects one. Rust's newtypes, enums,
   and typestate all compile away, so this costs nothing at runtime.
5. **We reuse existing patterns and make code reusable.** Smaller functions,
   smaller modules, one owner per concept.
6. **Clean code, DDD, and TDD.** A behavior change starts with a failing test.
7. **Respect the user's rate limit and their terminal.** Conditional requests
   and backoff are not optimizations; they are correctness. A crash must restore
   the terminal.

## NEVER: real identities anywhere in this repository

**No real account name, organisation, repository name, pull request title or
CI job name goes into this repository. Anywhere.** Not in code, not in tests,
not in fixtures, not in documentation, not in a README example, not in a commit
message.

Use the examples the crate already uses: `octocat`, `hubot`, `monalisa`,
`octodev`, `example-org/example-repo`. Casing matters where a test turns on it —
`OCTOCAT` against `octocat` — so do not flatten a pair to one spelling.

This has been violated three times: recorded fixtures, then the core's own
doctests, then a README written *after* both were cleaned. Each fix needed a
history rewrite, and a rewrite is only cheap while nothing is published. The
rule is absolute because the cost is asymmetric: using `octocat` costs nothing,
and using a colleague's login costs a `filter-repo` run and a force-push, or is
permanent.

Before any commit that adds an example, a fixture, or documentation:

```bash
git grep -niE "your-org|a-colleague|real-repo-name"   # must return nothing
```

Recording a fixture goes through `crates/prtrack-github/tests/fixtures/record.sh`,
which redacts by default. Never hand-edit a fixture.

## Skills location

- Canonical project skills live in `.agents/skills`.
- `.claude/skills` and `.codex/skills` are symlinks to `../.agents/skills`, so
  every host discovers the same skills.
- Add or update project skills only in `.agents/skills` to avoid drift.

### Superpowers workflows are a repo-vendored LOCAL plugin (not global)

- The `superpowers:*` workflow skills (`brainstorming`, `writing-plans`,
  `subagent-driven-development`, `test-driven-development`, `executing-plans`,
  `systematic-debugging`, `requesting-code-review`,
  `finishing-a-development-branch`, etc.) come from the **repo-tracked**
  vendored plugin at `.agents/plugins/superpowers/` (forked from
  obra/superpowers v6.0.3, MIT, with local edits), registered as
  `superpowers@gh-pr-tracker-local` via the `gh-pr-tracker-local` marketplace
  in `.claude/settings.json`.
- `.agents/skills/superpowers` is a **symlink** into that plugin's `skills/`,
  so there is a single source with no copy that can drift. Edit the real files
  under `.agents/plugins/superpowers/skills/<name>/`.
- Invoking `superpowers:<name>` loads this **local, canonical** content. Do not
  treat the `superpowers:` prefix as "global".

## Project skills

| Skill | Use for |
| --- | --- |
| `rust-best-practices` | Canonical Rust rules. Load before writing or reviewing any `.rs` file. |
| `code-review` | The multi-agent review gate. Run before `/verify`. |
| `verify` | The full verification suite. Definition of done before pushing. |
| `merge-main-and-verify` | Merging `origin/main` into a standalone branch. |
| `github-stacked-pr` | Native stacked PRs. The only place a rebase is allowed. |
| `fix-comments` | Fetch, fix, and reply to PR review comments. |
| `skill-creator` | Authoring or editing any skill under `.agents/skills`. |

Routing between the two branch workflows is exclusive:

```text
Standalone branch:                merge-main-and-verify
Confirmed native stack:           github-stacked-pr
```

## NEVER SKIP: Definition of Done

A change is done only when the PR head commit has green CI and no unresolved
review comments. Pushing is one step in the loop, not the end of it.

1. Behavior change → failing test first.
2. `/code-review` — the soft gate. Triage findings against the authorized scope.
3. `/verify` — format, clippy, build, tests, docs, and a real CLI smoke run.
4. Push, then watch CI: `gh run list --limit 1` / `gh run view <id>`.
5. `/fix-comments` for review feedback, then back to step 3.

CI is `.github/workflows/ci.yml`: a macOS gate carrying fmt, clippy, build,
tests, docs and a smoke run, plus a Linux job building the static musl artefact
the release ships. **Both must be green.** A macOS-only gate once let a Linux
build that never compiled sit on `main` for an hour.

A release is `.github/workflows/release.yml`, triggered by a `vN.N.N` tag whose
version must match `prtrack-cli`'s. Cut a release only from a commit whose CI is
already green, and remember the version bump — a tag ten minutes ahead of a fix
ships the bug.

`origin/main` is the stable baseline. If a gate surfaces a problem outside the
branch's authorized scope, do not silently fix it from the feature branch.
Report the concrete failure, explain why it appears unrelated, propose the
smallest fix or carve-out, and get explicit authorization.

## Working style

- **Branch responsibility.** You own the full scope of changes already in the
  branch and any failure they cause. You do not own unrelated `origin/main`
  behavior. Report, propose, get authorization — then fix.
- **Verify before asserting.** Never claim a test, CI, or code state ("passed",
  "pre-existing", "already fixed") from memory. Re-run the exact command or
  re-read the source and cite the evidence.
- **Surgical scope.** Make the smallest change that satisfies the request. No
  speculative features, abstractions, or refactors beyond the task. Clean up
  only what your change touched.
- **Illegal states, and how far to chase them.** New and rewritten types must
  make illegal states unrepresentable — not negotiable, no permission needed.
  For existing types the rule binds on **blast radius**, not on "the diff
  touched this file": fix a type your change constructs, or whose invariant your
  change depends on. A type you merely read stays as it is. When the fix would
  exceed roughly 10 files, stop and hand over a refactor: propose a branch,
  name the invariant being closed, and paste the call-site list the compiler
  produced. Do not file it as an issue — deferred type debt filed as an issue
  does not get done.
- **Aggressive DRY.** Before writing code, ask how the codebase already does
  this. Reuse it or extract a shared helper. This applies to types: reuse an
  existing type rather than adding a parallel one that exists only to be mapped
  into another shape.
- **No legacy without callers.** Do not leave `#[deprecated]` or dead code with
  no usages. Remove it, but confirm with `AskUserQuestion` first.
- **Mocking is a last resort.** Prefer extracting a pure function. Never mock a
  type this project owns.
- **Do not blindly agree.** If you disagree, say so via `AskUserQuestion`. Do
  not make decisions on the user's behalf when they have not responded.
- **Branch discipline.** Never commit directly to `main`. Use a feature branch
  or worktree and open a PR. The exception is an explicit instruction from the
  user, which has happened once, for a squash-merge.
- **Stage the files you edited, not the directory.** `git add crates/some-crate`
  is harmless until somebody else is working in that crate, at which point it
  silently absorbs their half-finished work into your commit. That is how a
  signature change landed without its call site and broke `main`.
- **Say so before editing a file scoped to another agent**, even when it is
  clean. Clean is a fact about the tree; announcing is a fact about the team,
  and only the second one stops two people reasoning about different histories.
- **No destructive git operation while uncommitted work exists in a shared
  tree.** `stash`, `checkout --`, `reset --hard` and `clean` are all destructive
  when somebody may write between your two steps. Commit first so a revert
  restores rather than destroys, or snapshot to `/tmp` with `cp`. To read a
  committed file without touching the tree, use `git show HEAD:path`.

## Code standards

### Writing code

Before writing or refactoring Rust, load `rust-best-practices`. It is the
canonical rule set (unwrap, panics, error types, borrowing, `pub` surface,
rustdoc, async discipline, function size) with worked examples. It is not
restated here.

**Governing principle: make illegal states unrepresentable.** See the
`Illegal States` rule category for newtypes, smart constructors, sum types, and
typestate.

Project-specific rules not covered by that skill:

- Never add `#[allow(clippy::...)]` to silence a warning. Fix the code, or raise
  the lint-level question as a config change. The one carve-out is generated
  code, which must carry a comment saying so.
- Never disable a lint or add `#[allow(dead_code)]` without user confirmation.
- Use constants over magic numbers. Check for an existing constant first.
- Extract shared types to a `types.rs` in the owning module.
- A GitHub API enum is never a bare `String`, and never gets a `_ =>` arm
  without an explicit `Unknown(String)` variant. GitHub adds values.

### Paths and structure

- Crates live under `crates/`. The library and the binary are separate crates.
- The `prtrack-cli` crate has both a `lib.rs` and a `main.rs`: the renderer and
  its friends live in the library half so they can be tested, and `main.rs` is
  wiring only. Nothing in that library is reusable outside this binary and it
  says so.
- Nothing terminal-related may be reachable from the library crate's public API.
- Docs live under `docs/`. Scripts live under `scripts/`. Do not create new
  top-level directories without saying why.
- `Cargo.lock` is committed. Every CI and verification command passes
  `--locked`.

### Errors

- Library: a `thiserror` enum, `#[non_exhaustive]`, with `#[source]` preserved.
- Binary: `anyhow` at the top level, with a readable error chain on stderr.
- No error `Display` text may contain a token, a URL with credentials, or an
  absolute path from a developer's machine.

### Output discipline

- Human-facing chatter and diagnostics go to **stderr**. Stdout is the data
  channel.
- Honor `NO_COLOR`, `CLICOLOR=0`, and a `--color` flag. Never emit ANSI codes
  when stdout is not a TTY.
- Never launch the interactive table when stdin/stdout is not a TTY. Fall back
  to a one-shot render.
- Exit codes are a contract. Distinguish at minimum: success, no results, usage
  error, auth failure, network/API failure.

### Testing

- Unit tests in `#[cfg(test)] mod tests` at the bottom of the file under test.
  Integration tests in `tests/`.
- No network access in tests. Use a recorded fixture or a local test server.
- Table-driven over copy-paste once there are three similar cases.
- A regression test that never went red proves nothing.

**Verify by mutation, not by reading.** Break the thing on purpose and watch a
test fail. Every vacuous test found in this project was invisible to review and
surfaced only this way. A test written specifically to close a gap, which does
not reach the gap, is worse than none: it converts an open question into a
settled one.

The recurring shapes, each of which cost a real bug here:

- **A probe already in normal form cannot test normalisation.** A fixture chosen
  to be tidy is thereby chosen to be inert.
- **When a test distinguishes two identifiers, the fixture must use the same
  text for both.** A team slug that cannot collide with a login passes whether
  the code compares accounts or rendered names.
- **A recorded fixture cannot cover a value the recorded repository does not
  produce.** Anything defined by an *absence* needs a synthetic case.
- **A two-reading fixture cannot tell "fired on the transition" from "fired on
  the condition."** A before and an after answer identically; the third reading
  is the one that discriminates.
- **A guard that cannot fire is worse than no guard**, because the code then
  lies about where the protection lives. Delete it rather than test around it.
- **Do not assert on a tool's phrasing.** `ldd` says "statically linked" where
  the guess is "not a dynamic executable"; `file` says "static-pie linked".
  Assert the property, not the sentence.

**"No real sleeps" forbids waiting, not measuring.** A test that sleeps until a
condition might have happened is slow and flaky in both directions. A test
asserting a *lower bound* on elapsed time, to prove a backoff actually slept,
can only be pushed further from failure by a loaded machine — that is allowed,
with the reason recorded beside it.

**`#[tokio::test(start_paused = true)]` and real socket IO are incompatible.**
Paused time auto-advances to the next timer whenever the runtime is idle, and a
test talking to a mock server is idle almost continuously — so the clock jumps
to the client's own request timeout and the request fails having never been
slow. Use virtual time only where nothing touches a socket; prefer restructuring
so the logic under test is a pure function over a fetch *outcome*.

**Run one mutation harness at a time.** It edits a file in place and restores it
afterwards, so it holds a whole-file lock that nothing enforces. Two at once
corrupt each other, and committing first does not help because both write after
the commit.

**A green gate describes the bytes on disk when its slowest step started.** If
anything may be writing to the tree, re-run against a clean `git archive`
export before believing it.
