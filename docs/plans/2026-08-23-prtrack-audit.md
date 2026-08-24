# prtrack-audit Implementation Plan

> **For Agents:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.
> Load `rust-best-practices` before writing any `.rs` file.

**Goal:** A distributable batch CLI that measures PR latency over a GitHub repo's recent history.

**Status note:** Stage 1's baseline is already measured. [`scripts/pr-latency.sh`](../../scripts/pr-latency.sh) is a working shell reference implementation, validated against 875 real PRs. **This Rust tool must reproduce its numbers exactly** — it is a distributable single binary a prospect can run without installing anything, not a different measurement. Treat the script as the executable specification and diff the two outputs in Task 11.

**Primary metric changed after measurement.** Settled wait (last push → first human review) replaces author self-block as the headline. Real data: settled wait p50 5.3h / p90 40.8h, author self-block p50 0.3h. Measuring from PR-open instead of last-push inflates p50 from 5.3h to 15.5h because authors keep pushing while they wait. Task 4 below is now the *second* metric, not the first.

**Architecture:** A pure metrics engine over normalised fact types, fed by a thin GitHub GraphQL client. The engine does no IO and is developed first, against hand-written fixtures. The network layer is added afterwards and converts wire types into domain types exactly once, at the boundary.

**Tech Stack:** Rust 2021, `reqwest` + `serde` (hand-written wire types), `chrono`, `clap`, `thiserror` in libraries, `anyhow` in the binary.

---

## Investigation evidence

There is no existing code — this repo contains agent tooling and documentation only (`git log`: two commits, no `.rs` files). The investigation therefore targets the **external contract**, which is where all the risk lives.

### Current path

Nothing implements this today. The design is
[`docs/plans/2026-08-23-pr-nudge-validation-design.md`](2026-08-23-pr-nudge-validation-design.md) §5.

### External contract findings that changed the plan

**1. Checks hang off commits, not pull requests, and only the head commit keeps them.**
Per [GitHub community discussion #70099](https://github.com/orgs/community/discussions/70099): *"Pull request checks are not set on pull requests; they are set on the last commit belonging to a pull request. If you push another commit, all the checks will disappear from that list."* Historical CI therefore requires paginating `commits(first: 100)` per PR, then `checkSuites` → `checkRuns(filterBy: {checkType: ALL})`. Without `checkType: ALL` re-runs collapse and earlier attempts are lost.

**2. That is expensive.** The `gh` CLI already [times out in large repositories](https://github.com/cli/cli/issues/7421) requesting far less nested check data than this would.

**3. Legacy commit statuses have no completion timestamp.** Only Check Runs expose `startedAt`/`completedAt`. Non-Actions CI reporting through the Status API gives `createdAt` only, so durations are unavailable for those.

**Consequence — the metrics are phased.** Two of the four metrics in the design doc are cheap; two are not:

| Metric | Data needed | Phase |
|---|---|---|
| Time to first review | PR + reviews + timelineItems | **A** |
| Approval → merge | PR + reviews | **A** (CI-agnostic proxy) |
| Idle ratio | PR + timelineItems | **A** |
| Red dwell | per-commit checkSuites → checkRuns | **B** |
| True author self-block (approved **and** green → merged) | per-commit check history | **B** |

Phase A needs one query per PR page. Phase B multiplies cost by commits per PR. **This plan covers Phase A only.** Phase A still answers "how bad is it in our repo," which is what Stage 1 exists to answer.

**4. Timeline union mechanics.** `timelineItems` is filtered by `itemTypes` using SCREAMING_SNAKE enum values, returns items chronologically, and requires `__typename` selection to discriminate. `requestedReviewer` is itself a union (`User | Team | Mannequin | Bot`) needing its own fragments. The older `timeline` field is deprecated. Draft intervals come from pairing `CONVERT_TO_DRAFT_EVENT` with `READY_FOR_REVIEW_EVENT`.

**5. Rate limit.** PATs get [5,000 points/hour](https://docs.github.com/en/graphql/overview/rate-limits-and-query-limits-for-the-graphql-api). Cost is computed by summing connection page sizes and dividing by 100. Every query in this tool must select `rateLimit { cost remaining resetAt }` so cost is measured rather than assumed.

### Reuse inventory

Nothing internal to reuse. Externally: `chrono` for time, `serde` for wire decoding, `clap` for CLI. No fixtures exist yet; Task 8 creates the first recorded one and later tasks extend it rather than adding parallel fixtures.

### Duplication decision

**`prtrack-provider` is deliberately NOT created.** The design doc lists a provider trait, but a trait with exactly one implementor is a speculative abstraction and fails the DRY gate. Extract it when GitLab is real and its shape is known from two APIs rather than one. This is a conscious deviation from the design doc.

**`prtrack-core` is deliberately NOT created yet.** The nudge rules engine has no caller until the product exists. The fact types live in `prtrack-audit` for now and get extracted into a shared crate when the second consumer appears. Extraction is mechanical; premature splitting is not.

### Bounded context

| Concept | Owner | Rationale |
|---|---|---|
| `PrNumber`, `Login`, `RepoRef`, `PullRequestFacts` | `prtrack-audit` | Domain vocabulary; no transport concerns |
| Metric definitions and computation | `prtrack-audit` | Domain invariants |
| GraphQL wire types, pagination, auth, rate limiting | `prtrack-github` | Transport only; must not acquire domain behaviour |
| Argument parsing, output formatting, exit codes | `prtrack-cli` | Shell |

Dependency direction is strictly `prtrack-cli → {prtrack-github, prtrack-audit}` and `prtrack-github → prtrack-audit` (for the `TryFrom` target types). `prtrack-audit` depends on neither.

### Scale contract

**Measured, not estimated.** A full run over `example-org/example-repo` — 875 merged PRs across four months, paginated 50 at a time with `reviews(first: 20)` and `timelineItems(last: 20)` — cost **1 point per page, ~18 points total** against a 5,000/hour budget.

The original estimate in this plan was 400–1,000 points. That was wrong by a factor of ~30, because GitHub divides the summed connection sizes by 100 and rounds, so modest page sizes collapse to a cost of 1. **Two consequences:** page sizes can be generous without penalty, and a hosted version could audit a large organisation for almost nothing — which materially improves the "connect your GitHub" lead-magnet idea.

Real constraint discovered instead: **the search API caps at 1000 results per query.** Beyond that, slice by merge date and run again. Never silently report a truncated population.

Memory is bounded by holding one page at a time and folding into an accumulator; do not collect all PRs before computing.

**Pagination is flaky.** One page in nine failed during the live run with a malformed response. Retry with backoff and verify `.data.search` exists before parsing; a dropped page silently shrinks the population.

### Rejected alternatives

| Option | Why rejected |
|---|---|
| `graphql_client` codegen | Requires checking in GitHub's multi-MB schema and adds a proc-macro build step. The field set here is ~20 fields on a very stable part of the schema. Revisit if the query set grows. |
| `octocrab` | Its strong typing is on the REST side; its GraphQL support is a raw-string passthrough. We would take a large dependency for an auth header. Reconsider for the product's REST notifications polling. |
| `cynic` | Plausible middle ground, but unverified maintenance status in current research. Not adopted on unverified evidence. |
| REST Actions API for CI timing | Richer for Actions specifically, but only covers Actions. Deferred with all of Phase B. |
| Collecting all PRs then computing | Unbounded memory for no benefit. Fold per page. |
| Business-hours-adjusted durations | Real credibility issue for the sales-artifact use (a Friday-evening PR inflates raw numbers), but Phase A's job is a consistent internal baseline where raw is fine. Named as a follow-up, not silently skipped. |

---

## Task 1: Workspace scaffold

**Files:**
- Create: `Cargo.toml`, `rust-toolchain.toml`, `.gitignore` (already exists — verify `/target` present)
- Create: `crates/prtrack-audit/Cargo.toml`, `crates/prtrack-audit/src/lib.rs`

**Step 1: Create the workspace root `Cargo.toml`**

```toml
[workspace]
resolver = "2"
members = ["crates/*"]

[workspace.package]
edition = "2021"
rust-version = "1.82"
license = "MIT"

[workspace.dependencies]
chrono = { version = "0.4", features = ["serde"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
unwrap_used = "warn"
expect_used = "warn"
panic = "warn"
wildcard_enum_match_arm = "warn"
```

Per `AGENTS.md`, each crate repeats `[lints]` rather than relying on inheritance.

**Step 2: Create `crates/prtrack-audit/Cargo.toml`**

```toml
[package]
name = "prtrack-audit"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
version = "0.1.0"

[dependencies]
chrono.workspace = true
serde = { workspace = true, optional = true }
thiserror.workspace = true

[lints]
workspace = true
```

**Step 3: Create `crates/prtrack-audit/src/lib.rs`**

```rust
//! Domain types and latency metrics for GitHub pull requests.
//!
//! This crate performs no IO. Everything here is a pure function of facts
//! supplied by a caller, which makes the metric definitions testable without a
//! network and portable to any consumer.

#![deny(missing_docs)]
```

**Step 4: Verify it builds**

Run: `cargo build --workspace --locked`
Expected: success, `Cargo.lock` created.

**Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/ rust-toolchain.toml
git commit -m "chore: scaffold cargo workspace with strict clippy standard"
```

---

## Task 2: Domain newtypes

Primitive obsession here would let a PR number and a check-run ID be interchangeable. See `rust-best-practices` § `newtype-for-domain-values`.

**Files:**
- Create: `crates/prtrack-audit/src/ids.rs`
- Modify: `crates/prtrack-audit/src/lib.rs`

**Step 1: Write the failing tests**

In `crates/prtrack-audit/src/ids.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pr_number_rejects_zero() {
        assert!(PrNumber::new(0).is_err());
    }

    #[test]
    fn pr_number_accepts_one() {
        assert_eq!(PrNumber::new(1).map(PrNumber::get), Ok(1));
    }

    #[test]
    fn login_rejects_blank() {
        assert!(Login::new("   ").is_err());
    }

    #[test]
    fn login_trims() {
        assert_eq!(Login::new(" octocat ").map(|l| l.as_str().to_owned()), Ok("octocat".to_owned()));
    }

    #[test]
    fn repo_ref_parses_owner_slash_name() {
        let r = RepoRef::parse("octocat/hello-world").expect("valid");
        assert_eq!(r.owner(), "octocat");
        assert_eq!(r.name(), "hello-world");
    }

    #[test]
    fn repo_ref_rejects_missing_slash() {
        assert!(RepoRef::parse("hello-world").is_err());
    }
}
```

**Step 2: Run to verify failure**

Run: `cargo test -p prtrack-audit`
Expected: FAIL — `cannot find type PrNumber in this scope`.

**Step 3: Implement**

Smart constructors with private fields, so an invalid value cannot be constructed.

```rust
//! Validated identifiers. Private fields plus checked constructors mean an
//! invalid identifier cannot exist at runtime.

use thiserror::Error;

/// Reason an identifier could not be constructed.
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum IdError {
    /// A pull request number must be at least 1.
    #[error("pull request number must be >= 1")]
    ZeroPrNumber,
    /// A login must contain at least one non-whitespace character.
    #[error("login must not be blank")]
    BlankLogin,
    /// A repository reference must be `owner/name`.
    #[error("repository must be in `owner/name` form")]
    MalformedRepo,
}

/// A pull request number, guaranteed non-zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PrNumber(u32);

impl PrNumber {
    /// Construct a pull request number.
    ///
    /// # Errors
    /// Returns [`IdError::ZeroPrNumber`] when `n` is zero.
    pub fn new(n: u32) -> Result<Self, IdError> {
        if n == 0 { Err(IdError::ZeroPrNumber) } else { Ok(Self(n)) }
    }

    /// The underlying number.
    #[must_use]
    pub const fn get(self) -> u32 { self.0 }
}
```

Implement `Login` (trimmed, non-blank) and `RepoRef` (exactly one `/`, both halves non-empty) to the same shape. Add `mod ids; pub use ids::*;` to `lib.rs`.

**Step 4: Run to verify pass**

Run: `cargo test -p prtrack-audit`
Expected: 6 passed.

**Step 5: Commit**

```bash
git add crates/prtrack-audit/src/
git commit -m "feat(audit): add validated PrNumber, Login and RepoRef newtypes"
```

---

## Task 3: Fact types and review state

`ReviewState` is a GitHub API enum, so per `AGENTS.md` it gets an explicit `Unknown(String)` variant and never a bare `_ =>` arm.

**Files:**
- Create: `crates/prtrack-audit/src/facts.rs`

**Step 1: Write the failing tests**

```rust
#[test]
fn unknown_review_state_is_preserved_not_dropped() {
    assert_eq!(
        ReviewState::from_api("SOMETHING_NEW"),
        ReviewState::Unknown("SOMETHING_NEW".to_owned())
    );
}

#[test]
fn known_review_states_parse() {
    assert_eq!(ReviewState::from_api("APPROVED"), ReviewState::Approved);
    assert_eq!(ReviewState::from_api("CHANGES_REQUESTED"), ReviewState::ChangesRequested);
}
```

**Step 2: Run to verify failure.** Expected: FAIL, `ReviewState` not found.

**Step 3: Implement**

```rust
/// A review's state as reported by the API.
///
/// `Unknown` exists because GitHub adds enum values; an unrecognised value must
/// stay visible rather than silently becoming a wrong known state.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReviewState {
    /// Approved the changes.
    Approved,
    /// Requested changes.
    ChangesRequested,
    /// Commented without an explicit verdict.
    Commented,
    /// Dismissed by a maintainer.
    Dismissed,
    /// Not yet submitted.
    Pending,
    /// A value this build does not recognise.
    Unknown(String),
}
```

Then the fact types. `PrOutcome` is an enum rather than a pair of `Option`s so "merged and closed-unmerged" is unrepresentable:

```rust
/// How a pull request ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PrOutcome {
    /// Still open at collection time.
    Open,
    /// Merged at the given instant.
    Merged(DateTime<Utc>),
    /// Closed without merging at the given instant.
    ClosedUnmerged(DateTime<Utc>),
}

/// Everything the metrics engine needs about one pull request.
#[derive(Debug, Clone)]
pub struct PullRequestFacts {
    /// Number within its repository.
    pub number: PrNumber,
    /// Who opened it.
    pub author: Author,
    /// When it was opened.
    pub created_at: DateTime<Utc>,
    /// When it first became non-draft, if it was ever a draft.
    pub ready_at: Option<DateTime<Utc>>,
    /// How it ended.
    pub outcome: PrOutcome,
    /// Review requests, chronological.
    pub review_requests: Vec<DateTime<Utc>>,
    /// Submitted reviews, chronological.
    pub reviews: Vec<ReviewFact>,
    /// All timeline activity timestamps, chronological. Used for idle time.
    pub activity: Vec<DateTime<Utc>>,
}
```

`Author` is an enum — `Human(Login)` or `Bot(Login)` — so bot filtering is a `match` rather than a string-suffix check scattered at call sites.

**Step 4: Run to verify pass.** Expected: 2 passed.

**Step 5: Commit**

```bash
git commit -am "feat(audit): add PR fact types with forward-compatible ReviewState"
```

---

## Task 4: Time to first review

The highest-value metric and the one with the most edge cases. Table-driven per `AGENTS.md`.

**Definition, stated explicitly because the number is meaningless otherwise:**

- **Clock starts** at the first review request; if there was none, at `ready_at`; if never a draft, at `created_at`.
- **Clock stops** at the earliest review whose author is not the PR author and whose state is not `Pending`.
- `None` if no qualifying review exists.
- A review submitted *before* the clock start yields `Duration::zero()`, never a negative.

**Files:**
- Create: `crates/prtrack-audit/src/metrics/first_review.rs`
- Test: same file, `#[cfg(test)] mod tests`

**Step 1: Write the failing table-driven test**

```rust
#[test]
fn time_to_first_review_cases() {
    struct Case {
        name: &'static str,
        facts: PullRequestFacts,
        expect_hours: Option<i64>,
    }

    let cases = vec![
        Case { name: "simple: requested then reviewed 3h later", facts: /* … */, expect_hours: Some(3) },
        Case { name: "draft: clock starts at ready_at, not created_at", facts: /* … */, expect_hours: Some(2) },
        Case { name: "self-review by author does not stop the clock", facts: /* … */, expect_hours: None },
        Case { name: "pending review does not stop the clock", facts: /* … */, expect_hours: None },
        Case { name: "no reviews at all", facts: /* … */, expect_hours: None },
        Case { name: "no explicit request falls back to created_at", facts: /* … */, expect_hours: Some(5) },
        Case { name: "review before request clamps to zero", facts: /* … */, expect_hours: Some(0) },
    ];

    for case in cases {
        let got = time_to_first_review(&case.facts).map(|d| d.num_hours());
        assert_eq!(got, case.expect_hours, "case: {}", case.name);
    }
}
```

Build the `facts` values with a `PullRequestFactsBuilder` test helper in `crates/prtrack-audit/src/testing.rs` behind `#[cfg(test)]`. Every later metric test reuses this builder — do not create a parallel one.

**Step 2: Run to verify failure.** Expected: FAIL, function not found.

**Step 3: Implement `time_to_first_review`.**

**Step 4: Run to verify pass.** Expected: 7 cases pass. The case name must appear in any failure message.

**Step 5: Commit**

```bash
git commit -am "feat(audit): compute time to first review with draft and self-review handling"
```

---

## Task 5: Approval to merge

**Definition:** from the `submittedAt` of the last `Approved` review preceding the merge, to `mergedAt`. `None` unless the outcome is `Merged` and at least one approval precedes it.

**This is a CI-agnostic proxy for author self-block** and the report must label it as such. The true metric requires Phase B.

Table cases: approved then merged; approved, changes requested, approved again (use the last); merged with no approval; closed unmerged; approval after merge timestamp (data anomaly — clamp to zero and count it in a `anomalies` tally rather than silently absorbing it).

Same five steps. Commit: `feat(audit): compute approval-to-merge latency`.

---

## Task 6: Idle time

**Definition:** sort `activity` ascending, bound by clock start (as Task 4) and end (`mergedAt` / `closedAt` / collection time). Idle is the sum of gaps strictly greater than a threshold, default 4 hours. Report both the absolute idle duration and idle ÷ total lifetime.

The threshold is a parameter, not a constant buried in the function — it will be tuned once real data arrives.

Table cases: no gaps; one long gap; several sub-threshold gaps summing above it (must not count); activity outside the bounds (ignored); zero-length lifetime (guard the division).

Same five steps. Commit: `feat(audit): compute idle time and idle ratio`.

---

## Task 7: Aggregation

**Files:** `crates/prtrack-audit/src/metrics/aggregate.rs`

**Percentile definition — nearest-rank, no interpolation.** For a sorted, 1-indexed series of length `n`, `p(q)` is the element at index `ceil(q × n)`. Documented in the rustdoc because two tools disagreeing on percentile method is exactly the kind of thing that discredits a number in front of a buyer.

Aggregate over a `PrOutcome::Merged`-only population by default; open PRs have no end and would skew every distribution.

Test cases: empty input yields `None` rather than zero; single element; even and odd lengths; all-equal values.

Commit: `feat(audit): aggregate metrics with nearest-rank percentiles`.

---

## Task 8: GraphQL wire types and the boundary conversion

**Files:**
- Create: `crates/prtrack-github/` (Cargo.toml, `src/lib.rs`, `src/wire.rs`, `src/query.graphql`)
- Create: `crates/prtrack-github/tests/fixtures/one_page.json`

The query, per the union mechanics confirmed above — note `__typename` on every union and `rateLimit` for cost measurement:

```graphql
query PrPage($owner: String!, $name: String!, $after: String) {
  rateLimit { cost remaining resetAt }
  repository(owner: $owner, name: $name) {
    pullRequests(first: 25, after: $after, orderBy: {field: CREATED_AT, direction: DESC},
                 states: [MERGED, CLOSED, OPEN]) {
      pageInfo { hasNextPage endCursor }
      nodes {
        number createdAt mergedAt closedAt
        author { __typename login }
        reviews(first: 50) {
          nodes { submittedAt state author { __typename login } }
        }
        timelineItems(first: 100,
          itemTypes: [READY_FOR_REVIEW_EVENT, CONVERT_TO_DRAFT_EVENT,
                      REVIEW_REQUESTED_EVENT, PULL_REQUEST_COMMIT,
                      ISSUE_COMMENT, PULL_REQUEST_REVIEW]) {
          pageInfo { hasNextPage endCursor }
          nodes {
            __typename
            ... on ReadyForReviewEvent { createdAt }
            ... on ConvertToDraftEvent { createdAt }
            ... on ReviewRequestedEvent { createdAt }
            ... on PullRequestCommit { commit { committedDate } }
            ... on IssueComment { createdAt }
            ... on PullRequestReview { createdAt }
          }
        }
      }
    }
  }
}
```

**Step 1:** Record a real fixture — run the query once with `gh api graphql` against a real repo and save the response to `tests/fixtures/one_page.json`. Redact nothing; it is public PR metadata. **Hand-edited fixtures drift from reality — always regenerate, never edit.**

**Step 2:** Write a failing test that deserialises the fixture and converts it, asserting the resulting `PullRequestFacts` for one known PR.

**Step 3:** Implement wire structs and `TryFrom<WirePullRequest> for PullRequestFacts`. Per `rust-best-practices` § `try-from-at-boundaries`, wire types are `pub(crate)` and must not escape this crate.

**Step 4:** Run. **Step 5:** Commit `feat(github): decode PR page and convert to domain facts`.

**Nested-pagination hazard:** a PR with >100 timeline items or >50 reviews is truncated. Detect `pageInfo.hasNextPage` on the inner connections and record the PR number in a `truncated` list that the report prints. **Silent truncation looks identical to a fast repo** and would corrupt the baseline.

---

## Task 9: Paginated client

**Files:** `crates/prtrack-github/src/client.rs`

Requirements:

- Token from `GITHUB_TOKEN` / `GH_TOKEN` env or a `--token-file`. **Never an argv flag** — argv is world-readable via `ps`.
- Manual `Debug` impl on the client that redacts the token. A derived `Debug` leaks it into any error chain.
- Stop paginating once `createdAt` precedes the requested window.
- Accumulate and report total `rateLimit.cost`; abort with a clear error when `remaining` drops below 100.
- Retry on 5xx and secondary-rate-limit responses with exponential backoff and jitter; never retry 401/404/422.
- A request timeout, so a hung connection cannot stall the run.

Test with a local `wiremock` server. **No network in tests** per `AGENTS.md`.

Commit: `feat(github): add paginated client with rate-limit accounting`.

---

## Task 10: CLI and report

**Files:** `crates/prtrack-cli/` with an `audit` subcommand.

```
prtrack audit --repo owner/name --since 2026-04-23 [--format text|json]
```

Output must include: PR count, excluded bot count, truncated-PR count, anomaly count, and p50/p90 for each metric. `--format json` emits **no ANSI and no progress**; progress goes to stderr. Exit codes are distinct for success, no-results, usage error, auth failure, and API failure, per `AGENTS.md`.

Label approval-to-merge explicitly as *"CI-agnostic proxy"* in the text output. An unqualified number here would be quoted back at us later.

Commit: `feat(cli): add audit subcommand with text and json output`.

---

## Task 11: Real run

Run against the real repository over ~4 months. Record in `docs/plans/` alongside this file: the numbers, the measured rate-limit cost, the truncation and anomaly counts, and any metric whose definition turned out to be wrong once real data hit it.

**This is the deliverable.** Tasks 1–10 exist to produce this number.

---

## Follow-ups, explicitly deferred

- **Phase B metrics** — red dwell and true author self-block, requiring per-commit check history.
- **Business-hours adjustment** — a Friday-evening PR inflates raw duration. Needed before any number is shown to a prospect.
- **Inner-connection pagination** — currently detected and reported, not followed.
- **Extract shared fact types** into their own crate when the product becomes the second consumer.
- **Extract a provider trait** when GitLab is real.
