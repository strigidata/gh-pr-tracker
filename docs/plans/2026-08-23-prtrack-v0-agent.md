# prtrack v0 — the PR board

> **For Agents:** REQUIRED SUB-SKILL: Use superpowers:executing-plans.
> Load `rust-best-practices` before writing any `.rs` file.

**Supersedes** [`2026-08-23-prtrack-audit.md`](2026-08-23-prtrack-audit.md).
This document replaces an earlier draft of itself that described a one-shot
nudge model. That model was wrong: PR state moves, so a single notification per
PR is useless, and firing per-PR would have sent nineteen misdirected messages
on the first real run because the failing checks were not the authors' fault.

**Goal:** A binary that shows every open PR and the reason it cannot merge yet,
derived from the repository's own branch protection rules — then, on `--watch`,
notifies when that reason changes.

**Architecture:** A pure decision core over normalised facts and a policy,
wrapped by a query layer and a renderer. The core does no IO and takes `now` as
a parameter. Delivery is swappable because the core only emits state and
transitions.

**Tech stack:** Rust 2021, `reqwest` + `serde` (hand-written wire types),
`chrono`, `clap`, `notify-rust`, `thiserror` in libs, `anyhow` in the binary.

**The specification already exists and works.** [`scripts/pr-board.sh`](../../scripts/pr-board.sh)
and [`scripts/pr-board.jq`](../../scripts/pr-board.jq) implement all of this
against the live repo. **Treat them as the executable spec.** Task 8's
acceptance test is that the Rust output matches the script's on the same repo.

---

## Why the board, and why policy-driven

Measured on `example-org/example-repo`, 37 open PRs (see the
[design doc](2026-08-23-pr-nudge-validation-design.md) §2 for the latency
baseline):

**Only 4 of ~60 checks are required.** Painting all failures red misdiagnosed
three PRs. `#4225` had been waiting five days for a review while the board
blamed its author for an advisory failure. `#3449` is approved, has
advisory-only failures, conflicts with main, and has sat untouched for 21 days —
invisible until the rules were read.

**Silence in the policy means "we cannot tell", not "nothing matters."** When no
required checks are configured, or the PR targets a branch the rule does not
cover (a stacked PR aimed at another feature branch), every failing check is
treated as blocking.

## The decision core

### `PolicyRules`

From `branchProtectionRules` matching the default branch. All fields optional —
a repository may have no protection at all.

```
required_checks: Vec<CheckName>     // empty => fall back to "everything blocks"
required_approvals: u32
requires_conversation_resolution: bool
requires_code_owner_reviews: bool
requires_strict_status_checks: bool // false => "behind" does not block
dismisses_stale_reviews: bool
```

### Check classification

A PR is **governed** when its base ref matches the rule's pattern *and* the rule
lists required checks. For a governed PR, a failing check blocks only if its
name is in `required_checks`; otherwise it is **advisory**. For an ungoverned
PR, every failing check blocks.

Failing states: `FAILURE`, `ERROR`, `TIMED_OUT`, `STARTUP_FAILURE`,
`ACTION_REQUIRED`. Running states: `IN_PROGRESS`, `QUEUED`, `PENDING`,
`WAITING`, `EXPECTED`. Check runs report `conclusion`; status contexts report
`state`; both must be handled and neither is a bare `String`.

### `BlockedReason` — the ladder, first match wins

```
1  Draft
2  ChangesRequested { by: Vec<Login> }
3  RequiredCheckFailing { count }
4  UnresolvedThreads { count }        only when requires_conversation_resolution
5  ApprovedButConflicting             the invalidation case
6  ApprovedCiRunning
7  ReadyToMerge
8  CiRunning
9  WaitingForReview { from: Vec<Login> }
10 NobodyAssigned
```

This is an enum, not a string. Rendering is a separate concern from deciding.

### Sections

| Section | Membership | Sort |
|---|---|---|
| Your PRs | author == viewer | action urgency, then oldest push. Drafts last. |
| Need your review | viewer in requested reviewers, not a draft, not yours | reviewability, then oldest push |
| Other drafts | someone else's draft | newest first |

Action urgency: ready-to-merge, conflicting, changes-requested, required-check-
failing, unresolved-threads, CI-running, everything else, drafts.

Reviewability: a PR blocked on its own author is not reviewable however green
its CI is — those sort last and render dimmed. On the live repo that is the
difference between a section of thirteen rows and the four that are actionable.

## Crates

```
crates/
  prtrack-core/     PolicyRules · PrFacts · classify · BlockedReason · sections   (pure, no IO)
  prtrack-github/   query, wire types, TryFrom
  prtrack-cli/      render · board · watch
```

`prtrack-core` depends on nothing else and takes `now` as a parameter.

---

## Task 1: Workspace, lints, newtypes

Workspace `Cargo.toml` with the strict clippy standard repeated per crate per
`AGENTS.md`. Then `PrNumber`, `Login`, `RepoRef`, `CheckName`, `HeadSha` — smart
constructors, private fields, TDD.

`CheckName` and `HeadSha` are newtypes specifically because both are strings
that must never be interchangeable with each other or with a `Login`.

## Task 2: `PolicyRules`

TDD. Cases: full rule as this repo has it; no rule at all; rule present but
`requiresStatusChecks: false`; rule present with an empty required list.

The last two must both yield "no required checks", because the fallback depends
on it.

## Task 3: Check classification

`classify(checks, policy, governed) -> Classified { blocking, advisory, running, passing }`

TDD, and these cases are the ones that were wrong in the shell prototype's first
draft:

- governed, failing check on the required list → blocking
- governed, failing check not on the list → advisory
- **ungoverned → every failing check blocking** (the fallback)
- policy with empty required list → every failing check blocking
- status context vs check run, both failing → both counted
- a check both failing and re-running → counted once, as running
- unknown check state string → not silently dropped; treat as running and
  surface it

## Task 4: `BlockedReason`

TDD the ten-rung ladder above. One test per rung plus the precedence pairs that
actually bit: changes-requested outranks required-check-failing; unresolved
threads are skipped entirely when the policy does not require resolution;
approved-and-conflicting outranks ready-to-merge.

## Task 5: Sections and sort keys

TDD. Cases: your draft sorts below your non-drafts; a PR you authored *and* are
requested on appears only under Your PRs; a PR blocked on its author sorts below
a clean one in the review section even with better CI; someone else's non-draft
PR you are not requested on appears in no section but is counted.

## Task 6: GitHub query and wire types

Port the query from `scripts/pr-board.sh` verbatim, including the
`application/vnd.github.merge-info-preview+json` Accept header, which
`mergeStateStatus` still requires.

Wire types `pub(crate)`, converted once with `TryFrom`. Record a fixture from
the live repo; **no network in tests**. Regenerate fixtures, never hand-edit.

Measured cost is ~1 point per 50 PRs with aggregate counts, ~50 with full
context nodes. The budget is a non-issue; pagination reliability is not — one
page in nine failed during the latency audit, so retry with backoff and verify
`.data` before parsing.

## Task 7: Renderer

Column widths must measure **visible** width, ignoring ANSI. Getting this wrong
is what broke the shell prototype's alignment.

Honour `NO_COLOR`, `CLICOLOR=0`, and `--color`, and emit no ANSI when stdout is
not a TTY, per `AGENTS.md`.

The policy line renders first: what is enforced, so the reader can trust the
rest of the screen.

## Task 8: `prtrack board` and the acceptance test

```
prtrack board --repo owner/name [--format text|json]
```

**Acceptance:** run `scripts/pr-board.sh example-org/example-repo` and
`prtrack board --repo example-org/example-repo` and diff. Modulo the timestamp
line, they must match. If they differ, the Rust is wrong until proven otherwise
— the script is the validated artefact.

`--format json` emits no ANSI and no progress. Distinct exit codes for success,
no results, usage error, auth failure, API failure.

## Task 9: macOS notification spike

Five minutes, and it gates Task 10's design. A bare binary has no bundle
identifier and macOS **silently drops** the notification
([notify-rust#132](https://github.com/hoodie/notify-rust/issues/132)). Call
`set_application()` with a borrowed bundle id first; do not discard the
`Result`, since the error names the problem. A real `.app` comes later — note
that TCC permission is keyed to bundle id plus signature, so a stable ad-hoc
signature beats none.

## Task 10: `--watch`

Redraw in place on an interval. Persist the previous snapshot, diff it, and
notify on **transitions that matter**, not on a timer:

- became `ReadyToMerge`
- became `ApprovedButConflicting` ← the invalidation case, currently invisible
- a required check went red
- you were newly requested as a reviewer
- approval dismissed by a push (needs `dismisses_stale_reviews`)

State file written atomically (temp + rename); prune entries for closed PRs.
Token from `GITHUB_TOKEN`/`GH_TOKEN`, never argv. Manual `Debug` on the client
that redacts it.

---

## Known gaps, deliberately unresolved

- **`dismisses_stale_reviews` is read and displayed but not acted on.** The
  board cannot yet distinguish "never approved" from "was approved until someone
  pushed". Given `#3449`, that distinction is worth having.
- Comment counts do not indicate whether a discussion is *settled*. A truer
  signal — last comment by someone other than the author, and no push since —
  is computable but is a feature, not a column tweak.
- Rulesets (the newer mechanism) are ignored. This repo has one, disabled, so
  classic branch protection is authoritative. A repo using rulesets only would
  currently read as unprotected and fall back to "everything blocks", which is
  safe but wrong.
- Only the rule matching the default branch is consulted; other patterns are
  ignored and their PRs fall back.
