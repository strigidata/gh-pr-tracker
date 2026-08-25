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

**This was built against an executable specification, which has since been
deleted.** A shell prototype implemented all of this against the live repository
and every task was validated by diffing against it. That acceptance test
**passed**:
zero factual differences across every open pull request, an identical `policy`
object, and every surviving difference a tie position or a count that moved
while the two commands ran.

They were then deleted, because a second implementation of one concept drifts
and the drift is silent. Two consequences are worth knowing rather than
rediscovering:

- **The port no longer has an external check.** The GraphQL document is verified
  only against recordings produced by that same document, which is a drift
  detector rather than a proof. A query wrong in a way its own recording
  reflects can now be caught by nothing. If that matters again, the prototype is
  recoverable from git history — it was deleted, not lost.
- **The oracle was wrong nine times and the port was wrong none.** Every
  disagreement that was not a tie or a timing artefact resolved in the Rust's
  favour, the last one being `#4311`'s draft state. Retiring it is not a loss of
  authority; it never had authority, only a head start.

---

## Status, and what the transport review left open

Tasks 1–7 and 6b are done: the core, the ladder, the board, the transport, the
wire→domain conversion, and the renderer. **Task 8 — the CLI and the acceptance
diff — is the only thing between 264 passing tests and a binary anyone can run.**
Until it lands, `main.rs` is still the notification spike.

A 51-mutation review of `prtrack-github` returned *approve with changes*. Its
three Criticals are closed. What remains, in the reviewer's priority order:

| | | |
| --- | --- | --- |
| **C1** | real personal data in git history | closed — branch squashed, objects purged before any push |
| **C2** | the GraphQL document was verified by nothing | closed — all 8 query mutations now caught |
| **C3** | a spent rate limit was retried three times | closed — `RATE_LIMITED` is `Permanent` |
| **I5** | `UPDATED_AT` cursor can skip a pull request | open — fix by ordering on `CREATED_AT` |
| **I4** | five nested connections capped with no completeness check | open — `contexts` peaks at 76 of 100 |
| **I8** | an error-nulled field reads as "nothing there" | open — cheap now that `path` is typed |
| **I6** | five correct behaviours pinned by nothing | open |
| **I7** | a `CheckRun` that fails to parse becomes an unknown union member | open |

**I4 and I5 are the two remaining routes to a silently short population**, which
is the failure this crate exists to prevent and the one nothing downstream can
detect. Their consequences are asymmetric and all bad: a dropped attempt at a
required check is a false green, a dropped `latestReviews` entry loses a
`CHANGES_REQUESTED`, and a missed protection rule falls back to `unprotected`
where everything blocks.

> **C2 made the query stable, not correct.** The recordings were produced *by
> this query*, so the two agree by construction about anything a recording is
> silent on. A query that is wrong in a way its own recording reflects can only
> be caught by nothing, now that the external comparison has been retired.

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
9  WaitingForReview { from: Vec<Reviewer> }
10 NobodyAssigned
```

This is an enum, not a string. Rendering is a separate concern from deciding.

**`Reviewer` is a sum type, not a `Login`.** GitHub's `reviewRequests` returns a
`RequestedReviewer` union — `User`, `Team`, `Mannequin`, or `Bot`. A team has a
`slug` and no login, so forcing it into `Login` would either lose the request or
fabricate an account that does not exist:

```rust
enum Reviewer { User(Login), Team(TeamSlug), Bot(Login), Mannequin(Login) }
```

This also decides the "is the viewer a requested reviewer?" test: viewer
membership must match `Reviewer::User`, and a pending *team* request means the
viewer may be an implicit reviewer without appearing by name. v0 matches on the
user only and counts team requests toward "someone is assigned"; a PR whose only
request is a team must not render as `NobodyAssigned`.

Measured on the live repo: all 71 current review requests are `User`, so this is
not a bug we are hitting today. It is modelled correctly anyway because the API
returns a union, `requiresCodeOwnerReviews` is enabled so a CODEOWNERS team can
appear at any time, and the alternative is a lossy `String`.

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

> **Run [`scripts/check-states.sh`](../../scripts/check-states.sh) against any
> repository prtrack is pointed at, and occasionally against this one.** It
> reads the recognised vocabulary out of `CheckState::parse` so it cannot drift,
> reports what the repository actually produces, and exits 2 on anything
> unrecognised. An unknown *terminal* state renders as "CI running" forever
> because no later attempt is coming, and neither the board nor the unit tests
> can see it — which is exactly how `CANCELLED` survived. This is the cheapest
> bug detector this module has.
>
> **The check-state vocabulary earlier in this plan is incomplete, and the
> merge rule was wrong.** `CANCELLED` is the fourth most common state on the
> target repository (45 occurrences) and was missing; `STALE` is in the same
> class. Both count as **failing** — a check that never reached success has not
> passed.
>
> More importantly, **one check name reports many times and only the newest
> attempt is current.** 9 of 38 open PRs carry a duplicated check name, 54
> duplicated name-groups between them, so this
> is the norm. Merging by outcome precedence produces false results in both
> directions — verified live: `Validate PR Title` going `FAILURE -> SUCCESS`
> read as a failing *required* check, and a superseded `CANCELLED` beside a real
> verdict masked it entirely. **Merge by recency.** Note GitHub's timestamps are
> not reliably ordered (`#4324`: `startedAt 21:24:07`, `completedAt 21:24:06`),
> so the tie-break must be deliberate and documented.
>
> **There is no "list every required check" API, and that is deliberate.**
> `RequiredChecks::as_slice()` exists only inside `BlockingChecks::Only`. Under
> `Every` the required set is genuinely *unknowable* — the fallback fires
> precisely because the policy told us nothing — so the answer is absent, not
> empty. If you are building "a required check that never reported", the `Every`
> branch has no answer to give and must not be treated as an empty list.


`classify(checks, policy, governed) -> Classified { blocking, advisory, running, passing }`

TDD, and these are the cases that are easiest to get wrong:

- governed, failing check on the required list → blocking
- governed, failing check not on the list → advisory
- **ungoverned → every failing check blocking** (the fallback)
- policy with empty required list → every failing check blocking
- status context vs check run, both failing → both counted
- a check both failing and re-running → counted once, as running
- unknown check state string → not silently dropped; treat as running and
  surface it

## Task 4: `BlockedReason`

> **Live transposition hazard, unfixed.** `Requirement` is one type shared by
> three fields — `conversation_resolution`, `code_owner_reviews`,
> `up_to_date_branch`. An enum no more stops you writing `NotRequired` where
> `Required` belongs than a `bool` stops `false` where `true` belongs, so those
> three swap silently and compile. Distinct newtypes per question would close
> it; the type count was settled at four and this was accepted rather than
> overlooked. Reconsider if a transposition ever actually bites.
>
> **Transposition hazard.** `PolicyRules::required_approvals` is a bare `u32`,
> and this task introduces an actual approval count — a second bare `u32` that
> swaps with it silently. Newtype one or both before they meet.


TDD the ten-rung ladder above. One test per rung plus the precedence pairs that
actually bit: changes-requested outranks required-check-failing; unresolved
threads are skipped entirely when the policy does not require resolution;
approved-and-conflicting outranks ready-to-merge.

## Which field is authoritative, and a mutation-testing caveat

**`mergeable` decides conflict; `merge_state` decides behind.** The two overlap —
`MergeState::Dirty` and `MergeableState::Conflicting` can disagree — and both
implementations converged on this split independently: `decision.rs` reads
`MergeableState` through `merge_readiness` and touches `merge_state` only in
`is_behind`. Recorded so nobody re-opens it. `merge_state` has no other
consumer.

**A `--lib`-only mutation run mislabels accessor coverage — read the *next*
run with this in mind, not the last one.** `Check::reported_at` is read
internally through the field, not the accessor, so a mutant returning
always-`None` dies only under `--doc`. **Use plain `cargo test`, not
`--all-targets`** — verified on a scratch crate: `--all-targets` runs lib, bins,
tests, benches and examples and *skips doctests entirely*, so following that
instruction would miss the exact mutant class this caveat exists to catch. Plain
`cargo test` reports two result lines; make exit code the authority, since a
doctest failure matches no `test module::` prefix a harness might grep for.

To be precise about what this does and does not qualify: the 48-mutant review
reported that mutant as a survivor and was **right** — at the time nothing
tested the accessor by any route, so it survived under every target. The caveat
only bites once coverage exists and lives in a doctest, which is now true of
`reported_at`. It qualifies future runs, not that finding.

**A mutation harness must never run concurrently with anything, including a
second copy of itself.** It edits a file in place and restores it afterwards, so
in practice it holds a whole-file lock that nothing enforces. Task 5 hit the
120-second timeout, backgrounded the run, then started a second in the
foreground: the results were garbage and one run's restore left the other's
mutant applied in the tree. Committing first — the rule below — does not save
you here, because both harnesses are writing after the commit. If a run is slow,
let it be slow, or split the mutant list and run the halves in sequence. This
was caught only because a test failed, and the test that caught it was one of
three written moments earlier to close a survivor.

**An expected literal in a test must not rely on a `\`-continuation.** `cargo
fmt` rewrites a multi-line string literal by joining the continuation, and the
stripped indentation comes back as literal spaces — so the formatter silently
*changes the value the test asserts against*. Task 7 was caught by its own test
immediately, but a formatter altering test data is the kind of thing that reads
as a flaky test rather than as what it is. Use single-line literals for anything
asserting on exact rendered text.

**Cite commit messages, not hashes.** Three hashes in this plan died in the
history rewrite, and a dead hash is worse than no citation because it looks like
evidence. Describe what the commit did; the message is greppable and survives a
rebase.

**Whenever a test distinguishes two identifiers, make the fixture use the same
text for both.** A probe whose team slug cannot collide with the viewer's login
passes whether the code compares accounts or rendered names — so it proves
nothing about the distinction it was written for. This has now been the survivor
three separate times, in three different modules. It is the same family as a
probe already in normal form: **the input cannot exercise the thing the test
names.**

**`start_paused` and real socket I/O are incompatible, not merely awkward.**
Paused time auto-advances to the next timer whenever the runtime has nothing
runnable, and a test talking to a mock server over a real socket spends most of
its life in exactly that state. The clock jumps to the nearest timer — which is
the client's own per-request timeout — and the request times out having never
been slow. Raising the timeout does not help; the clock jumps to wherever it is.
Use a virtual clock only where nothing touches a socket.

The detail worth carrying: of the two tests tried, the one with the *smaller
response body* passed. It was passing on how long the socket took to deliver a
fixture, which is passing by luck. **A test that survives on body size is not a
test**, and keeping one on virtual time and one on real would have left a green
assertion that meant nothing.

**"No real sleeps" forbids waiting, not measuring.** A test that sleeps until a
condition might have happened is the thing the rule exists to stop — it is slow
and it is flaky in both directions. A test that asserts a *lower bound* on
elapsed time to prove a backoff actually slept is measuring, and a loaded
machine can only push it further from failure. Two such tests cost ~300ms and
are accepted, with the reason recorded beside them.

**Stage the files you edited, not the directory.** `git add crates/some-crate`
is harmless for exactly as long as nobody else is in that crate, and the day
someone is, it silently absorbs their in-flight work into your commit. That is
how a signature change landed without its call site and broke HEAD. Same shape
as the destructive-git rule below: a command that is safe alone and dangerous in
company.

**And say so before editing a file scoped to another agent**, even when it is
clean. The coordinator added two accessors to a file it had scoped to someone
else, checked the file was uncommitted-clean first, and still cost an hour —
because the owner found the change by debugging a broken build rather than by
being told. Clean is a fact about the tree; announcing is a fact about the team.

**A green gate only describes the bytes on disk when its slowest step started.**
`cargo test` passed with 106 tests while `clippy` failed in the same minute: the
test binary had been built before an edit landed and clippy ran after. If
anything may be writing to the tree, a gate result is evidence about a tree that
may no longer exist. Re-run it against a clean `git archive` export when it
matters.

**Verify the thing the gate verifies.** `cargo clippy -p some-crate` without
`--all-targets` skips test code, so a lint in a test helper ships and the
crate-scoped check reports green. Narrowing the command and then reporting the
result as though it were the gate is how both of the above got through.

**No destructive git operation while uncommitted work exists in a shared tree.**
Two agents and the controller each lost work to a different command — `git
stash` whose pop then conflicted, and `git checkout --` inside a mutation
harness reverting work that had never been committed. "Avoid stash" is the wrong
lesson; the rule is that `stash`, `checkout --`, `reset --hard` and `clean` are
all destructive when someone else may write between your two steps. Commit
first so a revert restores rather than destroys, or snapshot to `/tmp` with
`cp`, which also survives another agent committing underneath you. To inspect a
committed file without touching the tree, use `git show HEAD:path`.

## Task 5: Sections and sort keys

> **Indexing caveat.** A `HashMap` keyed by `RepoRef` keeps the **first** key
> inserted and only replaces the value. Since `RepoRef` equality is now
> case-insensitive but `as_str()` preserves the spelling it was given, a
> user-typed `--repo example-org/EXAMPLE-REPO` arriving before the API's canonical
> `example-org/example-repo` leaves the board rendering the user's capitalisation
> indefinitely. Key the index from the API-sourced value, or use `entry`
> deliberately. Cosmetic rather than a lie, but non-obvious.
>
> **Constraint from Task 1, do not miss this.** `Login` and `RepoRef` have
> **case-insensitive** `Eq`/`Hash` — GitHub identity is case-insensitive, so
> `octocat == Octocat`. A *derived* `Ord` on either would compare the raw string
> and therefore contradict `Eq`, which misbehaves in `BTreeMap` and in any
> sort that assumes a total order consistent with equality. If sorting needs
> `Ord` on these, hand-write it over the ASCII-lowercased form. `PrNumber` and
> `CheckName` are unaffected.


TDD. Cases: your draft sorts below your non-drafts; a PR you authored *and* are
requested on appears only under Your PRs; a PR blocked on its author sorts below
a clean one in the review section even with better CI; someone else's non-draft
PR you are not requested on appears in no section but is counted.

**Done. The delivered contract, which Task 7 codes against:**

```rust
BoardRow::reduce(facts: PrFacts, policy: &PolicyRules) -> BoardRow
Board::assemble(rows: Vec<BoardRow>, viewer: &Login) -> Board
  ->  yours() needs_your_review() other_drafts() not_waiting_on_you() total()
row ->  facts() checks() reason() urgency() reviewability()
```

Two departures from the brief, both deliberate. **The row carries `Classified`,
not only the facts and the reason** — `reviewability` needs `total() == 0` for
the "no checks reported" case, which is not a ladder rung and cannot be read off
`BlockedReason`. Keeping it also closes a hazard: `reduce` derives checks *and*
reason from one policy, so no caller can pair checks classified under one rule
with a reason decided under another. **`reduce` takes a policy per row, not one
per board** — a stacked PR aimed at another feature branch is governed by
nothing, and resolving which rule applies needs branch patterns only the caller
has. `Board::assemble` therefore has no policy parameter.

> **`BehindBase` is second in urgency, not seventh. Do not map it from the rung
> number.** The ladder's rung 7 is gated *inside* the approval, so it never means
> "out of date and also unreviewed" — it always means "this is the last thing
> before the button", which is one click away from `Mergeable`. Ranking it by
> ladder position puts an approved, one-click PR below a failing one. Task 5 hit
> exactly this: a behind-but-unapproved fixture landed in `AwaitingReview`, which
> is correct, and reading the stale ordering out of the brief is what sent it to
> `decision.rs` to find out why.

## Task 6: GitHub query and wire types

> **`CheckStatusState::COMPLETED` has no counterpart in `CheckState`, and it is
> almost every check.** 1428 of 1430 check runs in the recorded fixtures carry
> it. Parsed naively it becomes `CheckState::Unknown`, which `classify` counts
> as *still running* — so every finished check would render as unfinished and
> the board would show the whole repository waiting on CI. The conversion must
> read `conclusion` first and fall back to `status`, treating a run with
> neither as pending. This is why the wire layer keeps three separate enums
> rather than reusing `CheckState`.
>
> **The query must fetch a timestamp per check.** Classification merges checks
> by name taking the newest attempt, so `startedAt` on `CheckRun` and
> `createdAt` on `StatusContext` are required fields, not optional extras.
> Without them the deserializer cannot build the input the core needs.
>
> **UNOWNED GAP — settle this before wiring real data.** The plan defines
> *governed* as "base ref matches the rule's pattern **and** the rule lists
> required checks". `PolicyRules` as built answers only the second half: it has
> no `pattern` field, so nothing stops `main`'s policy being applied to a
> stacked PR targeting a feature branch — exactly the case the plan says must
> fall back to "every failure blocks". Task 2 documented base-ref matching as
> the caller's job and left it there, correctly, since it needs a glob matcher
> and that is a real decision. **Whoever does Task 6 owns it**, or the
> stacked-PR fallback silently regresses to whatever `main` enforces.
>
> **Carry-over from Task 1.** GraphQL returns PR numbers as `Int` (i32), so the
> boundary needs `u32::try_from` before `PrNumber::new`. Rather than reconciling
> `TryFromIntError` with `InvalidValue` at each call site, add
> `TryFrom<i64> for PrNumber` in `prtrack-core` returning `InvalidValue` — that
> is what `try-from-at-boundaries` exists for. The variant was already renamed
> to `PrNumberOutOfRange` to cover it, but **its `Display` text still reads
> "must be greater than zero"** and must be widened in the same change, or an
> overflow will be reported as a zero.


The query needs the `application/vnd.github.merge-info-preview+json` Accept
header, which `mergeStateStatus` still requires.

Wire types `pub(crate)`, converted once with `TryFrom`. Record a fixture from
the live repo; **no network in tests**. Regenerate fixtures, never hand-edit.

Measured cost is ~1 point per 50 PRs with aggregate counts, ~50 with full
context nodes. The budget is a non-issue; pagination reliability is not — one
page in nine failed during the latency audit, so retry with backoff and verify
`.data` before parsing.

> **`hasNextPage: false` does not mean you have everything.** The recorder wrote
> 20 + 19 against a reported total of 41 and its own post-conditions passed it.
> Under an `UPDATED_AT` ordering a pull request touched between two requests
> moves to the front and the cursor steps straight over it; GitHub then reports
> the list as ended and the pair is quietly short. This is inherent to a cursor
> over a mutable ordering and the client cannot fix it — which is what
> `reported_total` beside `collected` is for. Compare them and refuse, rather
> than trusting the flag. It is the exact failure this crate exists to catch,
> produced by the tool that makes its fixtures.
>
> **Review targets for the transport layer, named by its author in advance.**
> Recorded because a self-review re-derives the same conclusions from the same
> assumptions and reads the agreement as confirmation. These are the four places
> where something was *decided* rather than followed:
>
> 1. **`data_of` accepts data whenever data is present, ignoring `errors`
>    alongside it.** If that is wrong the failure is silent — a response with a
>    real problem in an unvalidated field passes as success.
> 2. **429 and 403 are not retried**, against the packet's "retry with backoff".
>    If wrong, a transient secondary limit becomes a hard failure in a poll loop.
> 3. **A malformed page is retried, so a bad query is indistinguishable from a
>    flake** and burns the budget three times before reporting.
> 4. **The four accessors on `OpenPullRequests`** — invented API rather than API
>    anyone asked for, the category most likely to be wrong and least likely to
>    be questioned by its author.
>
> And the blind spot the author cannot see past: the tests encode *their reading
> of what a field means*. A round-trip guard proves fidelity to bytes, not to
> meaning, so if a field was misread every test agrees with the mistake. Review
> this field by field against something outside the implementation — not
> against its own intent.

## Task 7: Renderer

Column widths must measure **visible** width, ignoring ANSI. The easy mistake
is stripping the `[31m` while leaving the escape byte in the count, which makes
every coloured cell measure one cell too wide.

Honour `NO_COLOR`, `CLICOLOR=0`, and `--color`, and emit no ANSI when stdout is
not a TTY, per `AGENTS.md`.

The policy line renders first: what is enforced, so the reader can trust the
rest of the screen.

## Task 8: `prtrack board` — delivered, and how it was validated

```
prtrack board --repo owner/name [--format text|json] [--watch]
```

**The acceptance test passed and the oracle it ran against has been deleted.**
The Rust `--json` was diffed against the prototype's on the live repository: zero factual differences across every open pull request, an
identical `policy` object, and every surviving difference either a tie position
or a count that moved while the two commands ran.

Three things from that exercise are worth keeping, because they outlive the
script that prompted them.

> **A live oracle on a moving repository produces differences that are neither
> implementation being wrong.** The target went from 40 open pull requests to 39
> during one afternoon. A push, a merge or a check finishing between two calls
> is a real difference that says nothing about correctness — so a difference
> that does not survive a re-run is the repository moving, and only one that
> does is real. Any future comparison against a live system needs that rule
> attached, or it has an unmeasured false-positive rate and invites the
> expensive failure: chasing a phantom into correct code.

> **Rendered text cannot be diffed between runs.** It carries a wall clock and
> relative ages that legitimately change minute to minute. The `--json`
> projection deliberately contains nothing time-dependent, which is why it is
> the thing worth comparing — and why it stays free of rendering choices.

> **Being right is not the same as being fresher.** The port disagreed with the
> oracle on `#4311`'s draft state and was correct, but correct because its fetch
> was more recent rather than because its logic was better. Distinguishing those
> took a human judgement every time, which is the argument against automating a
> live comparison into a gate.

**What remains unverified, now that the oracle is gone.** The GraphQL document
is checked only against recordings produced by that same document — a drift
detector, not a proof. A query wrong in a way its own recording reflects is
caught by nothing. The prototype is recoverable from git history if that
matters again.


## Task 9: macOS notification spike

Five minutes, and it gates Task 10's design. A bare binary has no bundle
identifier and macOS **silently drops** the notification
([notify-rust#132](https://github.com/hoodie/notify-rust/issues/132)). Call
`set_application()` with a borrowed bundle id first; do not discard the
`Result`, since the error names the problem. A real `.app` comes later — note
that TCC permission is keyed to bundle id plus signature, so a stable ad-hoc
signature beats none.

## Task 9 findings — measured, and they change Task 10

**Borrowing the host terminal's bundle identifier is the wrong advice**, and this
plan gave it. On a modern terminal `__CFBundleIdentifier` is something like
`dev.warp.Warp-Stable`, which already talks to `usernoted` as a *modern*
`UNUserNotificationCenter` client. `notify-rust`'s default backend is the legacy
`NSUserNotification` one, one bundle id cannot be both, and the notification is
dropped — three runs of three, **exit 0 every time**:

```
Legacy client dev.warp.Warp-Stable connecting to modern client.
  You can't mix modern clients with legacy clients.
Denying message 3 from connection <LegacyConnection ...>
```

This gets worse the more modern the terminal: iTerm2, VS Code and Ghostty all
post their own notifications. `com.apple.Terminal` works because it stays on the
legacy path. The banner then carries **Terminal's name and icon**, and clicking
it activates Terminal.app.

**A Focus mode silently suppresses delivery and the process cannot tell.** For a
product whose entire value is the interruption this is a core failure mode, not
an environment quirk: an agent firing perfectly into a suppressed Focus session
is indistinguishable from a dead one. Verified with a control — `osascript`,
first-party and fully authorized, was suppressed identically. **Task 10 must
detect and surface this**, or `prtrack status` will report a healthy agent while
every nudge is being eaten.

**Ad-hoc signing is rejected.** `usernoted` logs `MacOS error: -67062`, verified
as `errSecCSUnsigned`. `codesign -dv` on the debug binary: `Signature=adhoc`,
`TeamIdentifier=not set`, `Info.plist=not bound`.

**Crate choice.** `user-notify` has the better macOS surface — buttons, inline
reply, click handling via `UNUserNotificationCenter` — but its Readme states it
works only inside a signed app bundle and cannot borrow one, so it would turn
the CLI's one working path into a no-op. `notify-rust` stays for the CLI. A
third option nobody has tested: `notify-rust`'s own `preview-macos-un` feature,
same modern API without a dependency swap, though it also needs a real bundle
and marks `set_application` as having no effect.

**Unverified, do not assume:** whether notification permission is keyed to
bundle id *plus signature*, such that re-signing resets the user's prompt.
Authorization lives in the usernoted store keyed by bundle id rather than in
`TCC.db`, and `usernoted` demonstrably reads the signature — which makes it
plausible and nothing more. A 20-minute experiment once a bundle exists, and it
should be run before anything depends on the answer.

**CLOSED — a banner was visually confirmed on 2026-08-24.** With both Focus
modes off and Terminal allowed in System Settings, the user saw it and the log
agrees: `interruptionSuppression: none`, `Delivering ... to [ .alert .lockScreen
.notificationCenter ]`, `Presenting ... as banner`. This is the first end-to-end
proof that an interruption can reach the developer, which is the premise the
whole product rests on.

**One prediction was wrong and the truth is better.** The borrowed bundle
identity was expected to make the banner read "Terminal". It does not: the title
is the summary we set, `prtrack`, and only the **icon** is Terminal's. So a real
signed `.app` bundle buys the correct icon rather than rescuing the name.

**Task 10 must detect suppression.** Both Focus modes were on for the first
attempt and the notification was silently dropped — a control with `osascript`,
first-party and fully authorised, was suppressed identically, so nothing was
being drawn from any source. A resident agent firing perfectly into a suppressed
session is indistinguishable from a dead one, which is the exact failure
`prtrack status` exists to make visible. Surface it beside the last-poll line.

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

## CI and the Definition of Done — no longer suspended

This section recorded that the repository had no remote and no CI, so the
Definition of Done could not be satisfied. Both now exist: the repository is
`strigidata/gh-pr-tracker`, `.github/workflows/ci.yml` gates every push on macOS
and Linux, and `.github/workflows/release.yml` cuts a release from a `vN.N.N`
tag. `AGENTS.md` is the current statement of the rule; this entry is kept only
so the gap in the record is explained rather than mysterious.


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
