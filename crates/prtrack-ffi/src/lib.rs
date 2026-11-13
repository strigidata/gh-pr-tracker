//! The boundary the macOS app links.
//!
//! A shell, like `prtrack-cli`: it fetches through `prtrack-github`, lets
//! `prtrack-core` judge, and projects the result into the two sections the
//! popover shows. Wording belongs to the Swift side; every decision belongs to
//! the core. Nothing here ranks, filters or re-classifies a row, and nothing
//! may depend on this crate.
//!
//! No `#![forbid(unsafe_code)]` here, unlike every other crate root: the
//! scaffolding `uniffi::setup_scaffolding!` and the `uniffi` derives generate
//! is `unsafe` FFI glue, and a forbid at the root would refuse it. Nothing
//! hand-written in this crate may use `unsafe`; its test and binary roots do
//! carry the forbid.

use std::error::Error;

use prtrack_core::{Board, Login, Placed, ReasonKind, RepoRef, Stacks};
use prtrack_github::{GitHubClient, RepositoryFacts};

uniffi::setup_scaffolding!();

/// `prtrack_core::ReasonKind`, declared to `UniFFI` rather than copied.
///
/// The derive matches every variant, so a rung added to the core's ladder
/// fails this crate's build until it is named here too.
#[uniffi::remote(Enum)]
pub enum ReasonKind {
    Draft,
    ChangesRequested,
    RequiredCheckFailing,
    UnresolvedThreads,
    ApprovedButConflicting,
    Conflicting,
    BehindBase,
    ApprovedCiRunning,
    ReadyToMerge,
    CiRunning,
    MergeabilityNotKnown,
    WaitingForReview,
    NobodyAssigned,
}

/// One row of the popover.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PopoverRow {
    /// The pull request's number.
    pub number: u32,
    /// What it is called.
    pub title: String,
    /// Who opened it; `None` for a deleted account.
    pub author: Option<String>,
    /// Where to open it.
    pub url: String,
    /// Why it cannot merge, as the viewer should read it: `Placed::reason`,
    /// never `BoardRow::reason`. On a re-review the two differ, and only the
    /// first sets aside the viewer's own change request the push answered.
    pub reason: ReasonKind,
    /// How many pull requests it is stacked on: 0 for a stack's root, 1 for
    /// what is stacked on that, and so on.
    pub depth: u32,
}

/// What the popover shows: who you are blocking, then your own work.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PopoverBoard {
    /// The account the token authenticates as, whose board this is.
    pub viewer: String,
    /// Somebody else's work waiting on the viewer, in the core's order.
    pub needs_your_review: Vec<PopoverRow>,
    /// The viewer's own open work, in the core's order.
    pub yours: Vec<PopoverRow>,
    /// Every open pull request the board was built from, including the
    /// sections the popover does not draw.
    pub total: u32,
}

/// Why there is no board. Never an empty one.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum FetchError {
    /// The fetch, or reading what came back, failed.
    #[error("{message}")]
    Failed {
        /// The failure and every cause beneath it, joined by `": "`. Carries
        /// no token: no error in the chain prints one.
        message: String,
    },
}

/// Fetches `repo`'s board with the token in `GH_TOKEN` or `GITHUB_TOKEN`.
///
/// # Errors
///
/// [`FetchError::Failed`] when there is no usable token, `repo` is not
/// `owner/name`, GitHub cannot be read, or what it sent cannot be represented.
#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_board(repo: String) -> Result<PopoverBoard, FetchError> {
    let client = GitHubClient::from_env().map_err(|error| failed(&error))?;
    fetch_board_with(&client, &repo).await
}

/// Fetches `repo`'s board with `client`.
///
/// Not exported over `UniFFI`: Swift never holds a client. Public so that tests
/// can point one at a local server.
///
/// # Errors
///
/// As [`fetch_board`], less the token.
pub async fn fetch_board_with(
    client: &GitHubClient,
    repo: &str,
) -> Result<PopoverBoard, FetchError> {
    let repo = RepoRef::parse(repo).map_err(|error| failed(&error))?;
    let open = client
        .fetch_open_pull_requests(&repo)
        .await
        .map_err(|error| failed(&error))?;
    let judged = RepositoryFacts::try_from(open)
        .map_err(|error| failed(&error))?
        .judge();
    Ok(project(judged.board(), judged.viewer()))
}

/// The two sections the popover draws, in the core's order.
fn project(board: &Board, viewer: &Login) -> PopoverBoard {
    PopoverBoard {
        viewer: viewer.as_str().to_owned(),
        needs_your_review: section(board.needs_your_review()),
        yours: section(board.yours()),
        total: count(board.total()),
    }
}

/// Every row of `stacks` in pre-order, each root followed by what sits on it.
fn section(stacks: &Stacks) -> Vec<PopoverRow> {
    stacks.walk().iter().map(row).collect()
}

/// One placed row, with the reason the viewer reads.
fn row(placed: &Placed<'_>) -> PopoverRow {
    let facts = placed.row().facts();
    PopoverRow {
        number: facts.number.get(),
        title: facts.title.as_str().to_owned(),
        author: facts.author.as_ref().map(|login| login.as_str().to_owned()),
        url: facts.url.as_str().to_owned(),
        reason: ReasonKind::of(placed.reason()),
        depth: count(placed.depth()),
    }
}

/// A count of pull requests, or a depth in a stack of them, as Swift reads it.
///
/// Saturating rather than fallible, because overflow is unreachable and a
/// guard that cannot fire would claim a protection that is not there. Both
/// numbers are bounded by the rows on the board: a depth is less than the rows
/// in its tree, and [`Board::total`] is the rows handed in. Through
/// [`fetch_board_with`] those rows are one fetch's pull requests, which the
/// client caps at a hundred pages of at most a hundred — under `u32::MAX` by
/// five orders of magnitude.
fn count(rows: usize) -> u32 {
    u32::try_from(rows).unwrap_or(u32::MAX)
}

/// `error` and every cause beneath it, as one line.
fn failed(error: &(dyn Error + 'static)) -> FetchError {
    let message = std::iter::successors(Some(error), |&error| error.source())
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ");
    FetchError::Failed { message }
}

#[cfg(test)]
mod tests {
    use super::project;
    use chrono::{DateTime, Utc};
    use prtrack_core::{
        Board, BoardRow, BranchName, Check, CheckName, CheckState, DraftState, HeadRepository,
        HeadSha, InvalidValue, Login, MergeState, MergeableState, Placed, PolicyRules, PrFacts,
        PrNumber, PrTitle, PrUrl, ReasonKind, RepoRef, Review, ReviewDecision, Verdict,
    };

    /// The commit the viewer reviewed: no longer the head.
    const EARLIER: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    /// The head the author pushed afterwards.
    const PUSHED: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    /// `monalisa`'s open pull request, green, on which `viewer` asked for
    /// changes at [`EARLIER`] before the author pushed [`PUSHED`].
    ///
    /// The change request is entered everywhere GitHub keeps it — the review,
    /// `changes_requested_by` and the review decision — because that stale fact
    /// is exactly what the row's own reason still reports and the viewer's
    /// reading sets aside.
    fn pushed_past_your_change_request(viewer: &Login) -> Result<PrFacts, InvalidValue> {
        Ok(PrFacts {
            repo: RepoRef::parse("example-org/example-repo")?,
            number: PrNumber::new(101)?,
            url: PrUrl::new("https://github.com/example-org/example-repo/pull/101"),
            title: PrTitle::new("Split the ledger writer"),
            author: Some(Login::parse("monalisa")?),
            draft_state: DraftState::ReadyForReview,
            head: HeadSha::parse(PUSHED)?,
            head_ref: BranchName::new("topic/ledger"),
            head_repository: HeadRepository::ThisOne,
            base_ref: BranchName::new("main"),
            review_decision: Some(ReviewDecision::ChangesRequested),
            mergeable: MergeableState::Mergeable,
            merge_state: MergeState::Clean,
            requested_reviewers: Vec::new(),
            changes_requested_by: vec![viewer.clone()],
            reviews: vec![Review {
                by: viewer.clone(),
                verdict: Verdict::ChangesRequested,
                on: Some(HeadSha::parse(EARLIER)?),
            }],
            unresolved_threads: 0,
            comments: 0,
            checks: vec![Check::new(
                CheckName::parse("build")?,
                CheckState::parse("SUCCESS"),
                Some(DateTime::<Utc>::UNIX_EPOCH),
            )],
            last_push: DateTime::<Utc>::UNIX_EPOCH,
        })
    }

    #[test]
    fn a_re_review_is_projected_with_the_reason_the_viewer_reads() -> Result<(), InvalidValue> {
        let viewer = Login::parse("octocat")?;
        let facts = pushed_past_your_change_request(&viewer)?;
        let board = Board::assemble(
            vec![BoardRow::reduce(facts, &PolicyRules::unprotected())],
            &viewer,
        );
        let walked = board.needs_your_review().walk();
        let placed = walked.first();
        assert_eq!(
            placed.and_then(Placed::rereview),
            Some(Verdict::ChangesRequested),
            "the fixture is a re-review in the review section"
        );
        let viewers = placed.map(|at| ReasonKind::of(at.reason()));
        let rows = placed.map(|at| ReasonKind::of(at.row().reason()));
        // Without this the assertion below could not tell the two apart.
        assert_ne!(viewers, rows, "the fixture discriminates the two reasons");

        let popover = project(&board, &viewer);

        let projected: Vec<_> = popover
            .needs_your_review
            .iter()
            .map(|row| (row.number, row.reason))
            .collect();
        assert_eq!(
            projected,
            Vec::from_iter(viewers.map(|reason| (101, reason)))
        );
        Ok(())
    }
}
