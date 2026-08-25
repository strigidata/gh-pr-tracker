//! The response to the client's query, exactly as GitHub sends it.
//!
//! # These types deliberately have no invariants
//!
//! Everywhere else this project prefers a type that cannot hold a wrong value.
//! Here the job is the opposite one: mirror what arrived, including the parts
//! that are wrong. A wire type that rejected a surprising value would turn one
//! odd pull request into a lost page, and a wire type that repaired one would
//! hide the repair from the code that has to decide whether the value can be
//! trusted.
//!
//! So the checking happens once, at the boundary, in the conversion into
//! `prtrack-core`'s domain types — which is a **later task and is not in this
//! crate yet**. What exists today stops at "the response deserialised".
//!
//! # Every field is written back out
//!
//! These types derive [`Serialize`] as well as [`Deserialize`], and the tests
//! at the bottom of this file read a recorded response, write it straight back
//! out, and compare the two as JSON. That is a stronger guarantee than spot
//! assertions: a field this module forgot to declare is dropped on the way out
//! and the comparison fails, so the types cannot silently know less than the
//! query asks for.
//!
//! Unknown *incoming* fields are still ignored, so a field GitHub adds does not
//! break a running build — it shows up the next time a fixture is re-recorded.

use serde::{Deserialize, Serialize};

// The GraphQL enums this query reads, one Rust enum per GitHub enum.
//
// # Why these are not [`prtrack_core::CheckState`]
//
// The core folds a check run's `conclusion`, a check run's `status`, and a
// status context's `state` into a single vocabulary, because nothing it
// decides depends on which of the three a value came from. That fold is a
// domain decision and it is lossy on purpose: `COMPLETED` is a perfectly good
// `CheckStatusState` and means nothing at all in the core's vocabulary, where
// it would land in `Unknown` and count as "still running". It is safe there
// only because the conversion reads `conclusion` first and never looks at
// `status` on a finished run.
//
// A wire type that had already folded could not offer the conversion that
// choice. So these mirror GitHub's three separate enums, and the fold happens
// once, at the boundary, where it can be read and tested.
//
// # An unrecognised value keeps its text
//
// GitHub adds enum values. Every enum here carries an `Unknown` variant holding
// the value exactly as it arrived, so a new one is neither a parse failure that
// loses the whole page nor a silent mapping onto the nearest known value.
//
// `Unknown` is the catch-all here because that is what it is in every enum of
// `prtrack-core`, and where GitHub itself spells a value `UNKNOWN` this crate
// renames it exactly as the core does — `Computing`, `Undetermined`. One word
// cannot mean "GitHub is still thinking" in one crate and "nobody could read
// this" in the next: the conversion between them is a `match`, and the arm
// that looks right has to be the one that is right.

/// Whether GitHub can merge the pull request as it stands —
/// [`MergeableState`](https://docs.github.com/en/graphql/reference/enums#mergeablestate).
///
/// Computed lazily. It is reliable for open pull requests, which is all this
/// query asks for; on a merged one it is always `UNKNOWN`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum MergeableState {
    /// The branch merges cleanly.
    Mergeable,
    /// The branch conflicts with its base.
    Conflicting,
    /// GitHub has not finished working out which of the two it is.
    ///
    /// **GitHub spells this `UNKNOWN`.** It is renamed for the same reason
    /// `prtrack_core::MergeableState::Computing` is, and to the same word: the
    /// catch-all below is called `Unknown`, and one enum holding two kinds of
    /// unknown is a bug waiting for a `match` arm. Keeping the two crates on
    /// one vocabulary means the obvious arm in the conversion is also the
    /// correct one.
    #[serde(rename = "UNKNOWN")]
    Computing,

    /// A value GitHub added after this was written.
    ///
    /// Distinct from [`MergeableState::Computing`], which is GitHub's own word
    /// for "still being worked out". Reading one as the other would report a
    /// pull request as computing forever, or a pull request GitHub is still
    /// thinking about as a value nobody can read.
    ///
    /// Held as the text that arrived, so a renderer can say what it could not
    /// read and the value writes itself back out unchanged. A newtype rather
    /// than `prtrack_core`'s `Unknown { reported }` because an untagged variant
    /// has to capture a bare JSON string; the name and the meaning are the
    /// same, so the conversion arm is a one-liner.
    #[serde(untagged)]
    Unknown(String),
}

/// Why the merge button is or is not green —
/// [`MergeStateStatus`](https://docs.github.com/en/graphql/reference/enums#mergestatestatus).
///
/// Only available under the `merge-info-preview` Accept header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum MergeStateStatus {
    /// The head is out of date with the base.
    Behind,
    /// A protection rule is holding the merge back.
    Blocked,
    /// Nothing is in the way.
    Clean,
    /// The branch conflicts with its base.
    Dirty,
    /// The pull request is a draft.
    Draft,
    /// A pre-receive hook must run first.
    HasHooks,
    /// GitHub could not work the state out.
    ///
    /// **GitHub spells this `UNKNOWN`**, renamed for the reason given on
    /// [`MergeableState::Computing`], and to the word
    /// `prtrack_core::MergeState::Undetermined` uses.
    #[serde(rename = "UNKNOWN")]
    Undetermined,
    /// Mergeable, but with failing or pending checks.
    Unstable,

    /// A value GitHub added after this was written.
    ///
    /// Distinct from [`MergeStateStatus::Undetermined`], which is GitHub's own.
    ///
    /// Held as the text that arrived, so a renderer can say what it could not
    /// read and the value writes itself back out unchanged. A newtype rather
    /// than `prtrack_core`'s `Unknown { reported }` because an untagged variant
    /// has to capture a bare JSON string; the name and the meaning are the
    /// same, so the conversion arm is a one-liner.
    #[serde(untagged)]
    Unknown(String),
}

/// The verdict of the review process as a whole —
/// [`PullRequestReviewDecision`](https://docs.github.com/en/graphql/reference/enums#pullrequestreviewdecision).
///
/// Null on a pull request no rule asks to be reviewed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum ReviewDecision {
    /// Enough approvals have been given.
    Approved,
    /// At least one reviewer asked for changes.
    ChangesRequested,
    /// A review is required and has not been given.
    ReviewRequired,

    /// A value GitHub added after this was written.
    ///
    /// Held as the text that arrived, so a renderer can say what it could not
    /// read and the value writes itself back out unchanged. A newtype rather
    /// than `prtrack_core`'s `Unknown { reported }` because an untagged variant
    /// has to capture a bare JSON string; the name and the meaning are the
    /// same, so the conversion arm is a one-liner.
    #[serde(untagged)]
    Unknown(String),
}

/// What one review said —
/// [`PullRequestReviewState`](https://docs.github.com/en/graphql/reference/enums#pullrequestreviewstate).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum ReviewState {
    /// Written but not submitted.
    Pending,
    /// Submitted without a verdict.
    Commented,
    /// Submitted as an approval.
    Approved,
    /// Submitted asking for changes.
    ChangesRequested,
    /// Superseded by a later push, where the rule dismisses stale reviews.
    Dismissed,

    /// A value GitHub added after this was written.
    ///
    /// Held as the text that arrived, so a renderer can say what it could not
    /// read and the value writes itself back out unchanged. A newtype rather
    /// than `prtrack_core`'s `Unknown { reported }` because an untagged variant
    /// has to capture a bare JSON string; the name and the meaning are the
    /// same, so the conversion arm is a one-liner.
    #[serde(untagged)]
    Unknown(String),
}

/// What a status context, or a whole rollup, reports —
/// [`StatusState`](https://docs.github.com/en/graphql/reference/enums#statusstate).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum StatusState {
    /// The repository expects this context and has not received it.
    Expected,
    /// The context could not produce a result.
    Error,
    /// The context reported failure.
    Failure,
    /// The context has been created and has not reported.
    Pending,
    /// The context reported success.
    Success,

    /// A value GitHub added after this was written.
    ///
    /// Held as the text that arrived, so a renderer can say what it could not
    /// read and the value writes itself back out unchanged. A newtype rather
    /// than `prtrack_core`'s `Unknown { reported }` because an untagged variant
    /// has to capture a bare JSON string; the name and the meaning are the
    /// same, so the conversion arm is a one-liner.
    #[serde(untagged)]
    Unknown(String),
}

/// How far along a check run is —
/// [`CheckStatusState`](https://docs.github.com/en/graphql/reference/enums#checkstatusstate).
///
/// `COMPLETED` says only that the run finished; what it decided is in
/// [`CheckConclusionState`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum CheckStatusState {
    /// The run has been requested and not queued.
    Requested,
    /// The run is waiting for a runner.
    Queued,
    /// The run is going now.
    InProgress,
    /// The run finished. See the conclusion for what it decided.
    Completed,
    /// The run is held before it starts, as by a deployment gate.
    Waiting,
    /// The run has been created and has not reported.
    Pending,

    /// A value GitHub added after this was written.
    ///
    /// Held as the text that arrived, so a renderer can say what it could not
    /// read and the value writes itself back out unchanged. A newtype rather
    /// than `prtrack_core`'s `Unknown { reported }` because an untagged variant
    /// has to capture a bare JSON string; the name and the meaning are the
    /// same, so the conversion arm is a one-liner.
    #[serde(untagged)]
    Unknown(String),
}

/// What a finished check run decided —
/// [`CheckConclusionState`](https://docs.github.com/en/graphql/reference/enums#checkconclusionstate).
///
/// Null while the run is still going.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum CheckConclusionState {
    /// The run finished and is asking for a human.
    ActionRequired,
    /// The run ran out of time.
    TimedOut,
    /// The run was stopped before it could reach a verdict.
    Cancelled,
    /// The run reported failure.
    Failure,
    /// The run passed.
    Success,
    /// The run finished deliberately without a verdict.
    Neutral,
    /// The run decided it had nothing to do.
    Skipped,
    /// The run never got as far as running.
    StartupFailure,
    /// GitHub discarded the result before it could be used.
    Stale,

    /// A value GitHub added after this was written.
    ///
    /// Held as the text that arrived, so a renderer can say what it could not
    /// read and the value writes itself back out unchanged. A newtype rather
    /// than `prtrack_core`'s `Unknown { reported }` because an untagged variant
    /// has to capture a bare JSON string; the name and the meaning are the
    /// same, so the conversion arm is a one-liner.
    #[serde(untagged)]
    Unknown(String),
}

/// A GraphQL response: some data, some errors, or both.
///
/// Both fields are optional because GraphQL says so. A response with neither is
/// the malformed page this crate retries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Envelope {
    /// The query result, absent when the query could not be executed at all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) data: Option<Query>,
    /// What went wrong. Can arrive alongside partial data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) errors: Option<Vec<GraphQlError>>,
}

/// One entry of a GraphQL `errors` array.
///
/// `locations` and `extensions` are still dropped, so unlike the rest of this
/// module an error entry does not round-trip. Nothing reads them, and no
/// recorded fixture contains an `errors` array for the round-trip test to
/// notice — which is the same reason everything about this type is tested by
/// construction rather than from a recording.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct GraphQlError {
    /// GitHub's own description of what went wrong.
    pub(crate) message: String,
    /// Which kind of failure it was.
    ///
    /// Absent on a query GitHub could not validate, and on the timeout it
    /// answers 200 with. Present, and load-bearing, on a spent rate limit.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub(crate) error_type: Option<GraphQlErrorType>,
    /// Where in the response the failure happened.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) path: Option<Vec<PathSegment>>,
}

/// The kind of failure a GraphQL error reports.
///
/// Only the one value that changes what this crate *does* is named. GitHub
/// documents others, but this crate has evidence for the behaviour of exactly
/// one of them, and inventing retry policy for types nobody here has seen would
/// be guessing dressed as a table. Everything else keeps its text and is
/// retried, which is what an unrecognised failure was already worth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum GraphQlErrorType {
    /// The primary GraphQL rate limit is spent.
    ///
    /// Reported as HTTP 200 with an error-only body, which is why the status
    /// code guard never sees it. The window resets on the hour, so retrying
    /// inside a second cannot help and is how a client earns a secondary limit
    /// on top.
    #[serde(rename = "RATE_LIMITED")]
    RateLimited,

    /// Any other kind, kept as the text it arrived as.
    #[serde(untagged)]
    Unknown(String),
}

/// One step of a GraphQL error's path.
///
/// A path names a field, or an index into a list — `["repository",
/// "pullRequests", "nodes", 3]`. Modelling only the field names would fail to
/// read the errors that say which entry of a list went wrong, which are exactly
/// the ones worth reading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum PathSegment {
    /// A field name.
    Field(String),
    /// An index into a list.
    Index(i64),
}

/// The top level of the query.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Query {
    /// What this call cost and what is left. Null for some credential types.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rate_limit: Option<RateLimit>,
    /// Whoever the token belongs to.
    pub(crate) viewer: Viewer,
    /// Null when the repository does not exist, or when this token cannot see
    /// it. GitHub does not distinguish the two on purpose.
    pub(crate) repository: Option<Repository>,
}

/// What one call cost against the GraphQL rate limit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RateLimit {
    /// Points this call cost.
    pub(crate) cost: i64,
    /// Points left in the current window.
    pub(crate) remaining: i64,
}

/// The account the token authenticates as.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Viewer {
    /// The viewer's login, for deciding whose pull requests are whose.
    pub(crate) login: String,
}

/// The repository, its policy, and one page of its open pull requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Repository {
    /// The default branch, which is the one a protection rule usually names.
    pub(crate) default_branch_ref: Option<BranchRef>,
    /// The protection rules this token is allowed to read. A token without
    /// admin rights sees an empty list rather than an error.
    pub(crate) branch_protection_rules: Connection<BranchProtectionRule>,
    /// One page of open pull requests.
    pub(crate) pull_requests: Connection<PullRequest>,
}

/// A named git ref.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BranchRef {
    /// The branch name, without `refs/heads/`.
    pub(crate) name: String,
}

/// A yes-or-no answer from a protection rule.
///
/// A two-variant enum rather than a `bool` because a rule carries six of them
/// in a row, and six booleans side by side is the shape where two get
/// transposed and nobody notices — "checks must pass" and "a push dismisses
/// approvals" would swap in silence. The wire form is still a JSON boolean;
/// only the Rust name for it is different, and it writes back out as the
/// boolean it arrived as.
///
/// The single flags elsewhere in this module stay `bool`, because a field
/// called `is_draft` reads as the question it answers and there is nothing
/// beside it to be confused with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "bool", into = "bool")]
pub(crate) enum Flag {
    /// The rule demands it.
    Yes,
    /// It does not.
    No,
}

impl From<bool> for Flag {
    fn from(raw: bool) -> Self {
        if raw {
            Self::Yes
        } else {
            Self::No
        }
    }
}

impl From<Flag> for bool {
    fn from(flag: Flag) -> Self {
        matches!(flag, Flag::Yes)
    }
}

/// One branch protection rule, which is where "blocking" is defined.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BranchProtectionRule {
    /// The branch name pattern this rule governs.
    pub(crate) pattern: String,
    /// Whether the rule demands approving reviews at all.
    pub(crate) requires_approving_reviews: Flag,
    /// How many, when it does.
    pub(crate) required_approving_review_count: Option<i64>,
    /// Whether every review thread must be resolved before merging.
    pub(crate) requires_conversation_resolution: Flag,
    /// Whether checks must pass before merging.
    pub(crate) requires_status_checks: Flag,
    /// Whether the branch must also be up to date with its base.
    pub(crate) requires_strict_status_checks: Flag,
    /// The checks that can actually block a merge. Everything else runs and is
    /// ignored by the merge button.
    pub(crate) required_status_check_contexts: Option<Vec<Option<String>>>,
    /// Whether a code owner's approval is required.
    pub(crate) requires_code_owner_reviews: Flag,
    /// Whether a push throws away the approvals given before it.
    pub(crate) dismisses_stale_reviews: Flag,
}

/// A GraphQL connection, holding whichever of the three parts the query asked
/// for.
///
/// All three are optional because the query asks for different combinations in
/// different places — `pageInfo` only where paging matters, `totalCount` only
/// where a total is shown. Absent means "not asked for", and each is skipped on
/// the way out so a connection writes back exactly the keys it arrived with.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Connection<T> {
    /// How many entries exist in total, not just on this page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) total_count: Option<i64>,
    /// Where this page sits in the list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) page_info: Option<PageInfo>,
    /// The entries themselves. GraphQL allows both the list and each entry to
    /// be null, and this mirrors that rather than flattening it: a null entry
    /// means the list is short by one, which is not the same as an empty list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) nodes: Option<Vec<Option<T>>>,
}

/// Whether another page follows, and the cursor that fetches it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PageInfo {
    /// Whether GitHub says there is more.
    pub(crate) has_next_page: bool,
    /// The cursor to pass as `after`. GraphQL allows this to be null even when
    /// `has_next_page` is true; the client treats that combination as a
    /// malformed page rather than as the end of the list.
    pub(crate) end_cursor: Option<String>,
}

/// One open pull request and everything the board needs to explain it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PullRequest {
    /// The number in the URL.
    ///
    /// Signed and 64-bit because the wire format is a GraphQL `Int` and this
    /// layer does not narrow. Deciding that a number is out of range is the
    /// boundary conversion's job, and it can only report a value it received.
    pub(crate) number: i64,
    /// The title, as written.
    pub(crate) title: String,
    /// The web URL, for opening it.
    pub(crate) url: String,
    /// Whether it is a draft.
    pub(crate) is_draft: bool,
    /// When it was opened, as an ISO 8601 string.
    ///
    /// Timestamps stay as text here. Parsing one is a conversion, and a
    /// conversion that fails at this layer would cost the whole page rather
    /// than the one pull request whose timestamp was odd.
    pub(crate) created_at: String,
    /// The object id at the head of the branch.
    pub(crate) head_ref_oid: String,
    /// The branch it wants to merge into, which decides whether a rule governs
    /// it.
    /// The pull request's own branch. `String!`, so never null.
    pub(crate) head_ref_name: String,
    pub(crate) base_ref_name: String,
    /// Whoever opened it. Null when that account is gone.
    pub(crate) author: Option<Actor>,
    /// The verdict of the review process as a whole.
    pub(crate) review_decision: Option<ReviewDecision>,
    /// Whether it merges cleanly.
    pub(crate) mergeable: MergeableState,
    /// Why the merge button is or is not green.
    pub(crate) merge_state_status: MergeStateStatus,
    /// Who has been asked to review.
    pub(crate) review_requests: Connection<ReviewRequest>,
    /// The latest review from each reviewer.
    pub(crate) latest_reviews: Connection<Review>,
    /// The review threads, resolved or not.
    pub(crate) review_threads: Connection<ReviewThread>,
    /// How many comments there are. The query asks for no comment bodies.
    pub(crate) comments: CommentSummary,
    /// The head commit, and the checks that reported on it.
    pub(crate) commits: Connection<CommitNode>,
}

/// A GitHub account, reduced to the handle that identifies it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Actor {
    /// The account's login.
    pub(crate) login: String,
}

/// A request for someone to review.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReviewRequest {
    /// Who was asked. Null when the reviewer is no longer visible.
    pub(crate) requested_reviewer: Option<RequestedReviewer>,
}

/// Whoever a review was requested from.
///
/// GitHub's `RequestedReviewer` is a union, and it can grow members. The
/// fallback arm keeps the `__typename` of a member this crate does not know,
/// so a review requested from something new does not fail the page — it is
/// simply a reviewer that cannot be named yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "__typename")]
pub(crate) enum RequestedReviewer {
    /// A person.
    User {
        /// Their login.
        login: String,
    },
    /// A team, which stands for whichever of its members responds.
    Team {
        /// The team's slug within its organisation.
        slug: String,
    },
    /// A union member this crate does not know.
    #[serde(untagged)]
    Other(UnknownNode),
}

/// A union member no variant of this crate's mirror names.
///
/// Holding the `__typename` rather than discarding it is what lets a renderer
/// say *what* it could not read, and what lets this type write itself back out
/// unchanged.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct UnknownNode {
    /// The GraphQL type name, exactly as it arrived.
    #[serde(rename = "__typename")]
    pub(crate) typename: String,
}

/// One reviewer's latest review.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Review {
    /// What it said.
    pub(crate) state: ReviewState,
    /// Who said it.
    pub(crate) author: Option<Actor>,
    /// When, as an ISO 8601 string. Null on a review that was never submitted.
    pub(crate) submitted_at: Option<String>,
}

/// One review thread on the diff.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReviewThread {
    /// Whether someone marked it resolved.
    pub(crate) is_resolved: bool,
    /// Whether the lines it hangs off have since changed.
    pub(crate) is_outdated: bool,
}

/// The comment count, which is all the query asks for.
///
/// A connection with only a total, rather than a [`Connection`] with two empty
/// fields, so the type says what was requested.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommentSummary {
    /// How many comments the pull request has.
    pub(crate) total_count: i64,
}

/// One entry of the commit connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CommitNode {
    /// The commit itself.
    pub(crate) commit: Commit,
}

/// The head commit, and what CI said about it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Commit {
    /// When it was committed, as an ISO 8601 string.
    pub(crate) committed_date: String,
    /// The checks that reported on it. Null when none ever did, which is not
    /// the same as all of them passing.
    pub(crate) status_check_rollup: Option<StatusCheckRollup>,
}

/// Every check that reported on one commit, and GitHub's summary of them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StatusCheckRollup {
    /// GitHub's own one-word summary.
    pub(crate) state: StatusState,
    /// Every reported attempt. One check name can appear several times.
    pub(crate) contexts: Connection<CheckContext>,
}

/// One reported attempt at one check.
///
/// The two members of GitHub's `StatusCheckRollupContext` union say the same
/// thing in different words, and they are kept apart here rather than folded:
/// a check run splits its verdict across `conclusion` and `status`, and only
/// the conversion knows that reading `status` on a finished run gives
/// `COMPLETED`, which means nothing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "__typename")]
pub(crate) enum CheckContext {
    /// A GitHub Actions or App check run.
    #[serde(rename_all = "camelCase")]
    CheckRun {
        /// The check's name, which is what a required-check list is matched
        /// against.
        name: String,
        /// What it decided. Null while it is still going.
        conclusion: Option<CheckConclusionState>,
        /// How far along it is.
        status: CheckStatusState,
        /// When this attempt started, as an ISO 8601 string. Null on an attempt
        /// that never started.
        ///
        /// Not decoration: GitHub reports every attempt, so one name arrives
        /// several times, and without an instant there is no telling a
        /// superseded failure from a live one.
        started_at: Option<String>,
    },
    /// A commit status posted by an older-style integration.
    #[serde(rename_all = "camelCase")]
    StatusContext {
        /// The context name, which plays the part a check run's `name` plays.
        context: String,
        /// What it reported.
        state: StatusState,
        /// When it was posted, as an ISO 8601 string.
        created_at: String,
    },
    /// A union member this crate does not know.
    #[serde(untagged)]
    Other(UnknownNode),
}

/// The members of `StatusCheckRollupContextConnection` this crate names.
///
/// Kept beside the enum it describes, and pinned by a test that parses a
/// well-formed object of each and refuses to see it in the fallback arm — so
/// the list cannot drift away from the variants.
const KNOWN_CHECK_CONTEXTS: [&str; 2] = ["CheckRun", "StatusContext"];

/// The members of `RequestedReviewer` this crate names.
const KNOWN_REVIEWERS: [&str; 2] = ["User", "Team"];

impl CheckContext {
    /// The type name of a member this crate *does* name that nevertheless
    /// landed in the fallback arm.
    ///
    /// The fallback is untagged, which is what lets an unfamiliar union member
    /// survive — and also what makes this necessary. Untagged means serde tries
    /// the named variants, and on failure falls through. So a `CheckRun` whose
    /// own fields did not parse — `name` renamed, `status` gone — arrives as
    /// `Other { typename: "CheckRun" }`, which is a contradiction: this crate
    /// knows what a `CheckRun` is.
    ///
    /// Left alone it is worse than a parse error. Every check run would become
    /// a context with no readable state, every state would classify as *still
    /// running*, and a repository whose required checks are failing would
    /// render as one waiting for CI. That needs a schema change to fire — and
    /// surviving a schema change is the whole reason the fallback exists, so
    /// the one case it must not swallow is the one it was built to catch.
    pub(crate) fn misparsed(&self) -> Option<&str> {
        match self {
            Self::Other(node) if KNOWN_CHECK_CONTEXTS.contains(&node.typename.as_str()) => {
                Some(&node.typename)
            }
            Self::CheckRun { .. } | Self::StatusContext { .. } | Self::Other(_) => None,
        }
    }
}

impl RequestedReviewer {
    /// The type name of a reviewer this crate names that landed in the
    /// fallback arm. See [`CheckContext::misparsed`].
    pub(crate) fn misparsed(&self) -> Option<&str> {
        match self {
            Self::Other(node) if KNOWN_REVIEWERS.contains(&node.typename.as_str()) => {
                Some(&node.typename)
            }
            Self::User { .. } | Self::Team { .. } | Self::Other(_) => None,
        }
    }
}

/// Responses recorded from the live GitHub API, with the user data replaced.
///
/// Recorded and redacted in one command by
/// `tests/fixtures/record.sh OWNER/REPO`, using the same query text the client
/// sends, at a page size that makes the two pages the whole list. Recorded, not
/// written: a hand-made fixture drifts from what the API actually returns, and
/// being shaped exactly like a real response is the only reason these are worth
/// having. Regenerate them by re-running that script, never by editing them —
/// it checks that nothing leaked, that the shape is unchanged, and that the
/// redaction is deterministic.
///
/// Logins, titles, branch names and the owner/repository in a URL are
/// synthetic. Check names, enum values, timestamps, counts and cursors are as
/// GitHub sent them, because every one of those is something a test reads.
///
/// Between them they carry every enum value the repository produces, a check
/// run still in progress with a null `conclusion`, a status context beside
/// check runs on the same commit, 54 repeated check names across 9 pull
/// requests, and a branch protection rule with four required contexts.
///
/// The constants below are pinned to a particular recording; `record.sh`
/// prints their new values when it writes a new one.
///
/// The first of two pages, with another to follow.
#[cfg(test)]
pub(crate) const PAGE_1: &str = include_str!("../tests/fixtures/open-pull-requests-page-1.json");

/// The second page, fetched with the first one's cursor, ending the list.
#[cfg(test)]
pub(crate) const PAGE_2: &str = include_str!("../tests/fixtures/open-pull-requests-page-2.json");

/// The cursor the first page ends on, which is what the second was fetched
/// with. The client must send exactly this to replay the recording.
#[cfg(test)]
pub(crate) const PAGE_1_END_CURSOR: &str =
    "Y3Vyc29yOnYyOpK0MjAyNi0wOC0yMVQxOTowNzoyMFrPAAAAAQJb17g=";

/// How many open pull requests the repository had when these were recorded.
#[cfg(test)]
pub(crate) const TOTAL_OPEN: usize = 34;

/// How many of them are on each page.
#[cfg(test)]
pub(crate) const PER_PAGE: usize = 19;

#[cfg(test)]
mod tests {
    use super::{
        CheckConclusionState, CheckContext, CheckStatusState, Envelope, Flag, GraphQlErrorType,
        MergeableState, PathSegment, PullRequest, RequestedReviewer, PAGE_1, PAGE_1_END_CURSOR,
        PAGE_2, PER_PAGE, TOTAL_OPEN,
    };

    /// A recorded response, read back, or `None` if any part of it could not
    /// be. Nothing below asserts against a value this returned `None` for
    /// without first saying so.
    fn parse(raw: &str) -> Option<Envelope> {
        serde_json::from_str(raw).ok()
    }

    /// Every pull request on a page, with the nullable layers unwrapped.
    ///
    /// `None` if the envelope did not parse, if any layer down to the node
    /// list was absent, or if any entry in it was null. Each test that uses
    /// this pins the count first, so a `None` that slipped through as an empty
    /// list would fail rather than quietly prove nothing.
    fn pull_requests(raw: &str) -> Option<Vec<PullRequest>> {
        let nodes = parse(raw)?.data?.repository?.pull_requests.nodes?;
        nodes.into_iter().collect()
    }

    /// Every check attempt reported across a page.
    fn check_contexts(raw: &str) -> Vec<CheckContext> {
        pull_requests(raw)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|pull_request| pull_request.commits.nodes)
            .flatten()
            .flatten()
            .filter_map(|node| node.commit.status_check_rollup)
            .filter_map(|rollup| rollup.contexts.nodes)
            .flatten()
            .flatten()
            .collect()
    }

    #[test]
    fn a_recorded_page_deserialises_with_its_values_intact() {
        let repository = parse(PAGE_1).and_then(|envelope| envelope.data?.repository);
        assert!(
            repository.is_some(),
            "the recording must parse into a repository or nothing below proves anything"
        );

        let connection = repository
            .as_ref()
            .map(|repository| &repository.pull_requests);
        assert_eq!(
            connection.and_then(|connection| connection.total_count),
            Some(i64::try_from(TOTAL_OPEN).unwrap_or(-1))
        );
        assert_eq!(
            repository
                .as_ref()
                .and_then(|repository| repository.default_branch_ref.as_ref())
                .map(|branch| branch.name.as_str()),
            Some("main")
        );

        let page_info = connection.and_then(|connection| connection.page_info.as_ref());
        assert_eq!(
            page_info.map(|info| info.has_next_page),
            Some(true),
            "the first of two pages must say another follows"
        );
        assert_eq!(
            page_info.and_then(|info| info.end_cursor.as_deref()),
            Some(PAGE_1_END_CURSOR)
        );
    }

    #[test]
    fn the_policy_the_whole_board_is_read_against_survives() {
        // Which checks can block is read from this rule. An empty list here
        // would make every failure look blocking, which is a different board.
        let rules: Vec<_> = parse(PAGE_1)
            .and_then(|envelope| envelope.data?.repository?.branch_protection_rules.nodes)
            .unwrap_or_default()
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(
            rules.len(),
            1,
            "the recording carries exactly one rule, on `main`"
        );

        let rule = rules.first();
        assert_eq!(rule.map(|rule| rule.pattern.as_str()), Some("main"));
        assert_eq!(
            rule.map(|rule| rule.requires_status_checks),
            Some(Flag::Yes)
        );
        assert_eq!(
            rule.map(|rule| rule.requires_conversation_resolution),
            Some(Flag::Yes)
        );
        assert_eq!(
            rule.and_then(|rule| rule.required_approving_review_count),
            Some(1)
        );
        assert_eq!(
            rule.and_then(|rule| rule.required_status_check_contexts.as_ref())
                .map(|contexts| contexts.iter().flatten().count()),
            Some(4),
            "the rule names four required checks"
        );
    }

    #[test]
    fn a_pull_request_keeps_the_fields_the_board_is_built_from() {
        let requests = pull_requests(PAGE_1).unwrap_or_default();
        assert_eq!(
            requests.len(),
            PER_PAGE,
            "the recording holds {PER_PAGE} pull requests, so anything else means one was lost"
        );

        let newest = requests.first();
        assert_eq!(newest.map(|request| request.number), Some(4449));
        assert_eq!(
            newest.map(|request| request.head_ref_oid.len()),
            Some(40),
            "a full object id came back"
        );
        assert_eq!(
            newest.map(|request| !request.title.is_empty() && !request.url.is_empty()),
            Some(true),
            "a pull request arrives with a title and a URL"
        );
        assert_eq!(
            newest.map(|request| request.author.is_some()),
            Some(true),
            "the author of an open pull request is present"
        );
        assert!(
            requests
                .iter()
                .any(|request| request.mergeable == MergeableState::Conflicting),
            "the recording holds conflicting pull requests, or the merge state was not read"
        );
        assert!(
            requests
                .iter()
                .any(|request| request.comments.total_count > 0),
            "the recording holds commented pull requests"
        );
    }

    #[test]
    fn both_kinds_of_check_survive_side_by_side_with_their_instants() {
        // A status context and a check run report the same thing in different
        // words, and an implementation that read only one kind would lose every
        // failure of the other. The instants are what make a repeated name
        // resolvable at all.
        let contexts = check_contexts(PAGE_1);
        assert!(
            contexts.len() > 500,
            "the recording holds hundreds of attempts, got {}",
            contexts.len()
        );

        let mut runs = 0_usize;
        let mut statuses = 0_usize;
        let mut in_flight = 0_usize;
        let mut unknown = 0_usize;
        for context in &contexts {
            match context {
                CheckContext::CheckRun {
                    name,
                    conclusion,
                    status,
                    started_at,
                } => {
                    runs += 1;
                    assert!(!name.is_empty(), "a check run arrives with a name");
                    assert!(
                        started_at.is_some(),
                        "every recorded attempt carries the instant that orders it"
                    );
                    if conclusion.is_none() {
                        in_flight += 1;
                        // Queued, waiting or running — but never `COMPLETED`,
                        // which with no conclusion would be a run claiming to
                        // have finished without saying what it decided.
                        assert_ne!(
                            *status,
                            CheckStatusState::Completed,
                            "a run with no conclusion has not finished"
                        );
                    }
                }
                CheckContext::StatusContext {
                    context,
                    created_at,
                    ..
                } => {
                    statuses += 1;
                    assert!(!context.is_empty() && !created_at.is_empty());
                }
                CheckContext::Other(_) => unknown += 1,
            }
        }

        assert_eq!(unknown, 0, "the recording holds no unknown context type");
        assert!(runs > 0 && statuses > 0, "both kinds are in the recording");
        assert!(
            in_flight > 0,
            "the recording holds a run still in progress, whose conclusion is null"
        );
    }

    #[test]
    fn a_repeated_check_name_arrives_repeated_rather_than_deduplicated() {
        // GitHub reports every attempt. If this layer collapsed them, the
        // conversion could not tell a superseded failure from a live one — the
        // bug that produced a false red on a required check.
        let names: Vec<String> = check_contexts(PAGE_1)
            .into_iter()
            .filter_map(|context| match context {
                CheckContext::CheckRun { name, .. } => Some(name),
                CheckContext::StatusContext { context, .. } => Some(context),
                CheckContext::Other(_) => None,
            })
            .collect();
        let mut distinct = names.clone();
        distinct.sort_unstable();
        distinct.dedup();

        assert!(
            distinct.len() < names.len(),
            "the recording repeats check names; got {} attempts under {} names",
            names.len(),
            distinct.len()
        );
    }

    #[test]
    fn a_recorded_page_writes_back_out_exactly_as_it_arrived() {
        // The strongest guard available here: a field these types forgot to
        // declare is dropped on the way out and the comparison fails. Spot
        // assertions cannot catch that, because a field nobody asserted on is
        // exactly the field nobody notices is missing.
        for (page, raw) in [("page 1", PAGE_1), ("page 2", PAGE_2)] {
            let written = parse(raw).and_then(|envelope| serde_json::to_value(&envelope).ok());
            let original = serde_json::from_str::<serde_json::Value>(raw).ok();
            assert!(
                written.is_some() && original.is_some(),
                "{page} must both parse and write back"
            );
            assert_eq!(
                written, original,
                "{page} lost or invented something on the way through"
            );
        }
    }

    #[test]
    fn the_two_recorded_pages_are_the_whole_list_with_nothing_repeated() {
        let mut numbers: Vec<i64> = [PAGE_1, PAGE_2]
            .into_iter()
            .filter_map(pull_requests)
            .flatten()
            .map(|request| request.number)
            .collect();
        assert_eq!(numbers.len(), TOTAL_OPEN);
        numbers.sort_unstable();
        numbers.dedup();
        assert_eq!(
            numbers.len(),
            TOTAL_OPEN,
            "a repeated number would mean pagination overlapped"
        );
    }

    #[test]
    fn a_graphql_error_keeps_the_type_that_says_which_failure_it_is() {
        // Recorded by hand from the live API, because no fixture can hold one:
        // a recording of a successful query has no `errors` array in it. This
        // is the exact envelope `owner=example-org-that-does-not-exist` returns.
        let raw = r#"{"data":{"viewer":{"login":"octocat"},"repository":null},
            "errors":[{"type":"NOT_FOUND","path":["repository"],
            "locations":[{"line":30,"column":3}],
            "message":"Could not resolve to a Repository with the name 'x/y'."}]}"#;
        let errors = parse(raw).and_then(|envelope| envelope.errors);
        assert_eq!(
            errors.as_ref().map(Vec::len),
            Some(1),
            "one error came back"
        );

        let first = errors.as_ref().and_then(|errors| errors.first());
        assert_eq!(
            first.and_then(|error| error.error_type.clone()),
            Some(GraphQlErrorType::Unknown("NOT_FOUND".to_owned())),
            "the type is what separates a spent rate limit from a flake"
        );
        assert_eq!(
            first.and_then(|error| error.path.clone()),
            Some(vec![PathSegment::Field("repository".to_owned())])
        );
    }

    #[test]
    fn a_rate_limit_error_is_read_as_the_rate_limit() {
        let raw = r#"{"errors":[{"type":"RATE_LIMITED","message":"exceeded"}]}"#;
        assert_eq!(
            parse(raw)
                .and_then(|envelope| envelope.errors)
                .and_then(|errors| errors.first().and_then(|e| e.error_type.clone())),
            Some(GraphQlErrorType::RateLimited)
        );
    }

    #[test]
    fn an_error_path_can_point_into_a_list() {
        // GraphQL paths carry list indices as numbers, so a path reading into
        // `pullRequests.nodes[3]` is integers mixed with strings. Modelling
        // only strings would fail to read the very errors that say which pull
        // request went wrong.
        let raw = r#"{"errors":[{"message":"boom",
            "path":["repository","pullRequests","nodes",3,"commits"]}]}"#;
        assert_eq!(
            parse(raw)
                .and_then(|envelope| envelope.errors)
                .and_then(|errors| errors.first().and_then(|e| e.path.clone())),
            Some(vec![
                PathSegment::Field("repository".to_owned()),
                PathSegment::Field("pullRequests".to_owned()),
                PathSegment::Field("nodes".to_owned()),
                PathSegment::Index(3),
                PathSegment::Field("commits".to_owned()),
            ])
        );
    }

    #[test]
    fn an_error_without_a_type_is_the_shape_a_timeout_arrives_in() {
        let raw = r#"{"errors":[{"message":"Something went wrong while executing your query."}]}"#;
        let first = parse(raw)
            .and_then(|envelope| envelope.errors)
            .and_then(|errors| errors.first().cloned());
        assert!(first.is_some(), "the envelope must parse");
        assert_eq!(first.as_ref().and_then(|e| e.error_type.clone()), None);
        assert_eq!(first.and_then(|e| e.path), None);
    }

    #[test]
    fn a_null_node_is_kept_as_a_hole_rather_than_dropped() {
        // GraphQL is allowed to null an entry it could not resolve. Dropping it
        // here would shrink the population silently, which reads as a
        // repository that merges faster than it does. It has to survive as a
        // hole so the client can refuse the page.
        let raw = r#"{"data":{"viewer":{"login":"octocat"},"repository":{
            "defaultBranchRef":{"name":"main"},
            "branchProtectionRules":{"nodes":[]},
            "pullRequests":{"totalCount":1,"pageInfo":{"hasNextPage":false,"endCursor":null},
                            "nodes":[null]}}}}"#;
        let nodes = parse(raw).and_then(|envelope| envelope.data?.repository?.pull_requests.nodes);
        assert_eq!(
            nodes.as_ref().map(Vec::len),
            Some(1),
            "the hole must still occupy its place in the list"
        );
        assert_eq!(
            nodes.map(|nodes| nodes.into_iter().all(|node| node.is_none())),
            Some(true),
            "the entry itself is the null one"
        );
        assert_eq!(
            pull_requests(raw).map(|requests| requests.len()),
            None,
            "and a caller collecting the list must not be handed a shorter one"
        );
    }

    #[test]
    fn a_known_member_whose_fields_did_not_parse_is_not_mistaken_for_an_unknown_one() {
        // The fallback is untagged, so it catches known members that failed to
        // parse as well as members nobody has heard of. A `CheckRun` calling
        // itself a `CheckRun` from inside the unknown arm is a contradiction,
        // and the quiet version of it is every check on the board reading as
        // still running.
        let renamed_field: Option<CheckContext> = serde_json::from_str(
            r#"{"__typename":"CheckRun","title":"gate","conclusion":"FAILURE",
                "status":"COMPLETED","startedAt":"2026-08-24T09:33:39Z"}"#,
        )
        .ok();
        assert_eq!(
            renamed_field.as_ref().and_then(CheckContext::misparsed),
            Some("CheckRun"),
            "a CheckRun that fell through must be reported, not accepted"
        );

        let renamed_reviewer: Option<RequestedReviewer> =
            serde_json::from_str(r#"{"__typename":"User","handle":"octocat"}"#).ok();
        assert_eq!(
            renamed_reviewer
                .as_ref()
                .and_then(RequestedReviewer::misparsed),
            Some("User")
        );
    }

    #[test]
    fn a_member_this_crate_has_never_heard_of_is_not_reported_as_misparsed() {
        // The fallback still has to do its job. A genuinely new union member is
        // not a parse failure, and refusing it would defeat the point.
        let unknown: Option<CheckContext> =
            serde_json::from_str(r#"{"__typename":"SomethingNew","state":"SUCCESS"}"#).ok();
        assert_eq!(unknown.as_ref().and_then(CheckContext::misparsed), None);

        let mannequin: Option<RequestedReviewer> =
            serde_json::from_str(r#"{"__typename":"Mannequin","login":"ghost"}"#).ok();
        assert_eq!(
            mannequin.as_ref().and_then(RequestedReviewer::misparsed),
            None
        );
    }

    #[test]
    fn every_named_member_parses_into_its_own_variant() {
        // Anti-drift for the two typename lists: they are written by hand
        // beside the enums, and a variant renamed without updating them would
        // leave a member that is neither matched nor reported. Each well-formed
        // object here must land in its named variant, never the fallback.
        let contexts = [
            (
                "CheckRun",
                r#"{"__typename":"CheckRun","name":"gate","conclusion":"SUCCESS",
                    "status":"COMPLETED","startedAt":"2026-08-24T09:33:39Z"}"#,
            ),
            (
                "StatusContext",
                r#"{"__typename":"StatusContext","context":"ci","state":"SUCCESS",
                    "createdAt":"2026-08-24T09:33:39Z"}"#,
            ),
        ];
        for (typename, raw) in contexts {
            let parsed: Option<CheckContext> = serde_json::from_str(raw).ok();
            assert_eq!(
                parsed.as_ref().and_then(CheckContext::misparsed),
                None,
                "a well-formed {typename} must parse into its own variant"
            );
            assert!(
                !matches!(parsed, Some(CheckContext::Other(_))),
                "a well-formed {typename} must not reach the fallback"
            );
        }

        let reviewers = [
            ("User", r#"{"__typename":"User","login":"octocat"}"#),
            ("Team", r#"{"__typename":"Team","slug":"platform"}"#),
        ];
        for (typename, raw) in reviewers {
            let parsed: Option<RequestedReviewer> = serde_json::from_str(raw).ok();
            assert!(
                !matches!(parsed, Some(RequestedReviewer::Other(_))),
                "a well-formed {typename} must not reach the fallback"
            );
        }
    }

    #[test]
    fn a_union_member_this_crate_does_not_know_keeps_its_type_name() {
        // Synthetic, not recorded: the live repository requests reviews only
        // from users, so no recording can cover a mannequin or a bot. GitHub's
        // unions do grow, and one new member must not cost the page.
        let reviewer: Option<RequestedReviewer> =
            serde_json::from_str(r#"{"__typename":"Mannequin","login":"ghost"}"#).ok();
        assert!(
            matches!(&reviewer, Some(RequestedReviewer::Other(node)) if node.typename == "Mannequin"),
            "expected the type name to survive, got {reviewer:?}"
        );

        let context: Option<CheckContext> =
            serde_json::from_str(r#"{"__typename":"SomethingNew","state":"SUCCESS"}"#).ok();
        assert!(
            matches!(&context, Some(CheckContext::Other(node)) if node.typename == "SomethingNew"),
            "expected the type name to survive, got {context:?}"
        );
    }

    #[test]
    fn a_team_reviewer_is_read_as_a_team_and_not_as_an_unknown() {
        // Also synthetic, and the reason the fallback above cannot be trusted
        // on its own: an arm that swallowed every union member would pass that
        // test and lose every team.
        let reviewer: Option<RequestedReviewer> =
            serde_json::from_str(r#"{"__typename":"Team","slug":"platform"}"#).ok();
        assert!(
            matches!(&reviewer, Some(RequestedReviewer::Team { slug }) if slug == "platform"),
            "expected a team, got {reviewer:?}"
        );
    }

    #[test]
    fn a_conclusion_github_adds_later_does_not_cost_the_page() {
        let context: Option<CheckContext> = serde_json::from_str(
            r#"{"__typename":"CheckRun","name":"gate","conclusion":"SOMETHING_NEW",
                "status":"COMPLETED","startedAt":"2026-08-24T09:33:39Z"}"#,
        )
        .ok();
        let conclusion = match &context {
            Some(CheckContext::CheckRun { conclusion, .. }) => conclusion.clone(),
            Some(CheckContext::StatusContext { .. } | CheckContext::Other(_)) | None => None,
        };
        assert_eq!(
            conclusion,
            Some(CheckConclusionState::Unknown("SOMETHING_NEW".to_owned())),
            "a check run with an unknown conclusion must still be a check run: {context:?}"
        );
    }
}

#[cfg(test)]
mod enum_tests {
    use super::{
        CheckConclusionState, CheckStatusState, MergeStateStatus, MergeableState, ReviewDecision,
        ReviewState, StatusState,
    };
    use serde::{de::DeserializeOwned, Serialize};

    /// Reads one GraphQL enum value, or `None` if it could not be read.
    fn read<T: DeserializeOwned>(value: &str) -> Option<T> {
        serde_json::from_str(&format!("\"{value}\"")).ok()
    }

    /// Reads a value and writes it straight back out, which is `None` if either
    /// half failed. A value that survives this cannot have been quietly mapped
    /// onto a different one.
    fn round_trip<T: DeserializeOwned + Serialize>(value: &str) -> Option<String> {
        let parsed: T = read(value)?;
        serde_json::to_string(&parsed)
            .ok()
            .map(|written| written.trim_matches('"').to_owned())
    }

    /// Every value GitHub's schema documents for the enums this query reads.
    ///
    /// Kept as one table so that a value added to the schema is added in one
    /// place, and so the "unrecognised" test below cannot accidentally use a
    /// value that is in fact real.
    const DOCUMENTED: [(&str, &[&str]); 7] = [
        ("MergeableState", &["MERGEABLE", "CONFLICTING", "UNKNOWN"]),
        (
            "MergeStateStatus",
            &[
                "BEHIND",
                "BLOCKED",
                "CLEAN",
                "DIRTY",
                "DRAFT",
                "HAS_HOOKS",
                "UNKNOWN",
                "UNSTABLE",
            ],
        ),
        (
            "ReviewDecision",
            &["APPROVED", "CHANGES_REQUESTED", "REVIEW_REQUIRED"],
        ),
        (
            "ReviewState",
            &[
                "PENDING",
                "COMMENTED",
                "APPROVED",
                "CHANGES_REQUESTED",
                "DISMISSED",
            ],
        ),
        (
            "StatusState",
            &["EXPECTED", "ERROR", "FAILURE", "PENDING", "SUCCESS"],
        ),
        (
            "CheckStatusState",
            &[
                "REQUESTED",
                "QUEUED",
                "IN_PROGRESS",
                "COMPLETED",
                "WAITING",
                "PENDING",
            ],
        ),
        (
            "CheckConclusionState",
            &[
                "ACTION_REQUIRED",
                "TIMED_OUT",
                "CANCELLED",
                "FAILURE",
                "SUCCESS",
                "NEUTRAL",
                "SKIPPED",
                "STARTUP_FAILURE",
                "STALE",
            ],
        ),
    ];

    /// The values documented for one enum, by the name used in [`DOCUMENTED`].
    ///
    /// Empty for a name the table does not hold, which the caller refuses.
    fn documented(name: &str) -> &'static [&'static str] {
        DOCUMENTED
            .iter()
            .find(|(key, _)| *key == name)
            .map_or(&[], |(_, values)| values)
    }

    /// Asserts that every value GitHub documents for `name` reads back as
    /// itself.
    fn every_documented_value_round_trips<T: DeserializeOwned + Serialize>(name: &str) {
        let values = documented(name);
        assert!(
            !values.is_empty(),
            "{name} is not in the table, so this test would prove nothing"
        );
        for value in values {
            assert_eq!(
                round_trip::<T>(value).as_deref(),
                Some(*value),
                "{name} must read {value:?} and write it back unchanged"
            );
        }
    }

    #[test]
    fn every_value_github_documents_reads_back_as_itself() {
        every_documented_value_round_trips::<MergeableState>("MergeableState");
        every_documented_value_round_trips::<MergeStateStatus>("MergeStateStatus");
        every_documented_value_round_trips::<ReviewDecision>("ReviewDecision");
        every_documented_value_round_trips::<ReviewState>("ReviewState");
        every_documented_value_round_trips::<StatusState>("StatusState");
        every_documented_value_round_trips::<CheckStatusState>("CheckStatusState");
        every_documented_value_round_trips::<CheckConclusionState>("CheckConclusionState");
    }

    /// A value GitHub has not defined for any of these enums today.
    const ADDED_LATER: &str = "SOMETHING_GITHUB_ADDED";

    #[test]
    fn the_value_used_to_test_the_unknown_case_is_not_a_real_one() {
        // Otherwise the test below would be asserting against a value some
        // enum genuinely knows, and would pass for the wrong reason.
        for (name, values) in DOCUMENTED {
            assert!(
                !values.contains(&ADDED_LATER),
                "{ADDED_LATER} is a real {name} value, so pick another"
            );
        }
    }

    #[test]
    fn a_value_github_adds_later_is_kept_rather_than_dropped_or_guessed_at() {
        // The whole page must survive a value this crate has never seen, and
        // the value must stay legible: mapping it onto the nearest known one
        // would turn an unreadable check into a pass, and failing the parse
        // would lose 49 pull requests because one of them was new.
        assert_eq!(
            round_trip::<MergeableState>(ADDED_LATER).as_deref(),
            Some(ADDED_LATER)
        );
        assert_eq!(
            round_trip::<MergeStateStatus>(ADDED_LATER).as_deref(),
            Some(ADDED_LATER)
        );
        assert_eq!(
            round_trip::<ReviewDecision>(ADDED_LATER).as_deref(),
            Some(ADDED_LATER)
        );
        assert_eq!(
            round_trip::<ReviewState>(ADDED_LATER).as_deref(),
            Some(ADDED_LATER)
        );
        assert_eq!(
            round_trip::<StatusState>(ADDED_LATER).as_deref(),
            Some(ADDED_LATER)
        );
        assert_eq!(
            round_trip::<CheckStatusState>(ADDED_LATER).as_deref(),
            Some(ADDED_LATER)
        );
        assert_eq!(
            round_trip::<CheckConclusionState>(ADDED_LATER).as_deref(),
            Some(ADDED_LATER)
        );
    }

    #[test]
    fn an_unrecognised_value_is_not_confused_with_githubs_own_unknown() {
        // `UNKNOWN` is a real value meaning "GitHub is still working it out",
        // and it is called `Computing` here for exactly that reason. A value
        // this crate cannot read is a different thing entirely, and it is the
        // one called `Unknown` — the same way round as in `prtrack-core`, so
        // the conversion's `Computing => Computing` and `Unknown => Unknown`
        // arms are both correct. Swapping them would render "still computing"
        // as "we cannot read this", or hide a value nobody has ever seen behind
        // a spinner that never stops.
        let githubs_own: Option<MergeableState> = read("UNKNOWN");
        let ours: Option<MergeableState> = read(ADDED_LATER);
        assert_eq!(githubs_own, Some(MergeableState::Computing));
        assert!(
            ours.is_some() && ours != githubs_own,
            "expected a distinct variant, got {ours:?}"
        );
    }
}
