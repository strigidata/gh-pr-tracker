# Re-review after a push — design and plan

## Problem

A pull request the viewer asked changes on stays in `YOU REVIEWED`, dimmed, with
the reason "changes requested by <viewer>", after the author has pushed new
commits in answer. GitHub keeps the review decision at `CHANGES_REQUESTED`
across a push, so the board reads it as waiting on the author when the next move
is the viewer's. Nothing nudges.

## Decisions (user, 2026-10-13)

- If the viewer's latest review — **changes requested or approved** — was made
  on a commit that is no longer the head, the pull request goes back to
  `NEED YOUR REVIEW` as a re-review, counts as reviewable, and `--watch` nudges
  once when that becomes true.
- The reason reads `RE-REVIEW: pushed since you asked for changes` or
  `RE-REVIEW: pushed since you approved`.

## Design

**Exact, not clock-based.** Each GitHub review records the commit it was made
on (`latestReviews { commit { oid } }`). A review is *stale* when that oid is
not the pull request's head. A review whose commit GitHub did not report is
*not* stale: a missing re-review is better than a false one (tenet 3), and the
recorded fixtures predate the field.

**Core facts** (`prtrack-core/src/facts.rs`). The carrier is enriched, not
paralleled: `PrFacts::reviewed_by: Vec<Login>` becomes
`PrFacts::reviews: Vec<Review>` with
`Review { by: Login, verdict: Verdict, on: Option<HeadSha> }` and
`Verdict { Approved, ChangesRequested }` (dismissed, commented and pending
reviews are still not "a say", as today). `PrFacts::rereview_due(&self, viewer)
-> Option<Verdict>` answers "has the head moved since this viewer's say?" —
the one predicate the board and the watcher both use. `changes_requested_by`
is unchanged.

**Board** (`board.rs`). In `Section::of`, after the requested-reviewer check, a
viewer whose review is stale lands in `NeedsYourReview`; an up-to-date one stays
`YouReviewed`. Each stack node records the re-review verdict and a
viewer-adjusted reason: `BlockedReason::decide` on the facts with the viewer's
own change request removed — "what would block this once my request is
answered". `Placed` exposes `rereview()` and `reviewability()` built from that
adjusted reason; board sorting, `awaiting_review` and a new `reviewable_now()`
count use it, so a re-review is not dimmed or sorted to the bottom by the
viewer's own stale request.

**Renderer** (`render.rs`). A re-review row's reason cell shows the RE-REVIEW
text; dimming and "reviewable now" use `Placed::reviewability()`.

**Watch** (`nudge.rs`, `watch.rs`, `state.rs`). `Watched` gains
`rereview: Rereview { Due, NotDue }` from `rereview_due`; `observe` emits
`Nudge::RereviewDue` on `NotDue → Due`. The stored state gains the field with
`#[serde(default)]` (a file written before it reads as `NotDue`; no format
version bump). `already_waiting` counts a due re-review as waiting on the
viewer.

**GitHub** (`prtrack-github`). Query `latestReviews` nodes gain `commit { oid }`;
wire `Review.commit: Option<ReviewCommit { oid }>` (serde default, skipped when
absent, for the round-trip test); conversion parses the oid with
`HeadSha::parse` (a malformed one refuses the pull request, as `head` does).
`oid` joins `ASKED_FOR_AFTER_THE_RECORDING`. The drift test
`both_queries_select_the_same_commit_fields` anchors on the first `commit {`,
which would now be the review's: re-anchor it on the head commit block.

**Versions.** `prtrack-core` 0.3.0 (`reviewed_by` replaced), `prtrack-cli`
0.1.9.

**Never** use real accounts, organisations, repositories, pull request numbers
or branch names — `octocat`, `hubot`, `monalisa`, `example-org/example-repo`,
PRs 101-106. Scratch output only in the repo's `.tmp/`, never `/tmp`.

## Tasks

1. **Facts and wire** (serial; touches every crate's test builders):
   `Review`/`Verdict`/`reviews`/`rereview_due` in core; query, wire,
   conversion, drift allowlist and re-anchored drift test in prtrack-github;
   `Section::of` switched to `reviews` with today's behaviour (any say →
   `YouReviewed`); every `reviewed_by` literal migrated. Tests: conversion of
   a review with an oid, without one, and with a malformed one;
   `rereview_due` table (no review, review on head, review on an earlier
   commit, review with no commit, approved vs changes requested).
2. **Board re-review** (lane A, `board.rs`): section, adjusted reason,
   `Placed::rereview`/`reviewability`, sorting, `awaiting_review`,
   `reviewable_now`, `you_have_reviewed`.
3. **Renderer** (lane A after 2, `render.rs`): reason text, dimming, summary.
4. **Watch nudge** (lane B, parallel with 2-3): `nudge.rs`, `watch.rs`,
   `state.rs`, including an older state file without the field.
5. **Finish**: versions, `/code-review`, `/verify`, smoke run, PR.

Each task is test-first, mutation-checked, and gets a spec and a quality
review before the next depends on it.

## Implementation review corrections

The change-request notice uses `RE-REVIEW: pushed since you blocked it` to fit
inside the existing reason column. A remaining change request from another
reviewer takes precedence in that column; the row still returns to the review
section and its own stale request is removed from the displayed blocker.

A saved state that predates re-review tracking has an uninitialized observation,
not evidence that no re-review was due. Its first live reading establishes the
baseline silently while preserving unrelated remembered transitions. Drafts and
the viewer's own pull requests do not request a re-review interruption. A push
and an explicit review request observed together produce one review alert.
