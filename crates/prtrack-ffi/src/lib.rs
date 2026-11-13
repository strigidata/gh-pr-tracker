//! The boundary the macOS app links.
//!
//! A shell, like `prtrack-cli`: it fetches through `prtrack-github`, lets
//! `prtrack-core` judge, and projects all six of the board's sections, and the
//! core's count of what is reviewable now, into records Swift can read.
//! Wording belongs to the Swift side; every decision belongs to the core.
//! Nothing here ranks, filters, re-classifies or re-counts a row: every row on
//! the board arrives in exactly one section, in the order the core drew it.
//! Nothing may depend on this crate.
//!
//! The static library and the Swift bindings generated from it are built and
//! shipped together, by `scripts/build-macos-spike.sh`. So a rung added to the
//! core's ladder is a compile error in Swift's exhaustive switches, never a
//! case an older app meets at run time. Bindings from a different build of this
//! library fail `UniFFI`'s checksum at the first call.
//!
//! No `#![forbid(unsafe_code)]` here, unlike every other crate root: the
//! scaffolding `uniffi::setup_scaffolding!` and the `uniffi` derives generate
//! is `unsafe` FFI glue, and a forbid at the root would refuse it. Nothing
//! hand-written in this crate may use `unsafe`; its test and binary roots do
//! carry the forbid.

use std::error::Error;

use prtrack_core::{Board, InvalidValue, Login, Placed, ReasonKind, RepoRef, Stacks, Verdict};
use prtrack_github::{ConversionError, GitHubClient, GitHubError, RepositoryFacts};

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

/// `prtrack_core::Verdict`, declared to `UniFFI` rather than copied.
///
/// The derive matches every variant, so a verdict added to the core fails
/// this crate's build until it is named here too.
#[uniffi::remote(Enum)]
pub enum Verdict {
    Approved,
    ChangesRequested,
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
    /// The viewer's own say the author has pushed past, if any:
    /// `Placed::rereview`. The CLI's reason column shows `reason` when it is
    /// [`ReasonKind::ChangesRequested`], otherwise the re-review sentence for
    /// this verdict when there is one, otherwise `reason`; a shell drawing
    /// `reason` alone tells a re-review it is waiting on nobody.
    pub rereview: Option<Verdict>,
    /// How many pull requests it is stacked on: 0 for a stack's root, 1 for
    /// what is stacked on that, and so on.
    pub depth: u32,
}

/// The whole board, section by section in the order the CLI draws them.
///
/// Each section holds the rows *drawn* there: every stack rooted in it, each
/// root followed by what is stacked on it, in pre-order. A stacked row is
/// drawn in its root's section rather than its own, so a section's rows are
/// not all rows that would sit there alone, and its length is not a count of
/// anything the viewer owes. [`PopoverBoard::reviewable_now`] is.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PopoverBoard {
    /// The account the token authenticates as, whose board this is.
    pub viewer: String,
    /// The rows drawn in the review section: stacks rooted on somebody else's
    /// work the viewer was asked to review, or must look at again since a
    /// push, with every row stacked on those roots. Most reviewable first.
    pub needs_your_review: Vec<PopoverRow>,
    /// The rows drawn under the review section: stacks rooted on somebody
    /// else's open work nobody asked the viewer about and the viewer has not
    /// reviewed, with every row stacked on those roots. Most reviewable first.
    pub unclaimed: Vec<PopoverRow>,
    /// The rows drawn in the reviewed section: stacks rooted on somebody else's
    /// work the viewer already approved or asked changes on, with every row
    /// stacked on those roots — including a review request of the viewer's
    /// that sits on one. By the root's number.
    pub you_reviewed: Vec<PopoverRow>,
    /// The rows drawn in the viewer's own section: stacks rooted on the
    /// viewer's open, non-draft work, with every row stacked on those roots.
    /// Most urgent first.
    pub yours: Vec<PopoverRow>,
    /// The rows drawn in the viewer's drafts: stacks rooted on the viewer's
    /// own drafts, with every row stacked on those roots. Newest push first.
    pub your_drafts: Vec<PopoverRow>,
    /// The rows drawn in the other drafts: stacks rooted on somebody else's
    /// draft, with every row stacked on those roots. Newest push first.
    pub other_drafts: Vec<PopoverRow>,
    /// How many rows are worth the viewer opening now, counting each row where
    /// it would sit alone rather than where it is drawn. This is the badge.
    /// No section's length is: a review request stacked on work the viewer
    /// already reviewed is drawn in `you_reviewed` and still counted here.
    pub reviewable_now: u32,
    /// Every open pull request the board was built from: the rows of all six
    /// sections together.
    pub total: u32,
}

/// Why there is no board. Never an empty one.
///
/// Grouped by what the person reading it should do, not by where it arose:
/// fix the token, fix the repository name, wait, or try again. Every variant
/// carries the same `message`: the failure and every cause beneath it, joined
/// by `": "`. It carries no token, because no error in the chain prints one.
///
/// `#[non_exhaustive]` binds Rust callers only. The Swift `UniFFI` generates is
/// the same with or without it, and Swift's switches over it stay exhaustive,
/// because the library and its bindings ship together.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
#[non_exhaustive]
pub enum FetchError {
    /// Neither `GH_TOKEN` nor `GITHUB_TOKEN` holds a token that can be sent.
    #[error("{message}")]
    NoToken {
        /// The failure and every cause beneath it.
        message: String,
    },
    /// GitHub refused the token: HTTP 401, or a 403 that is not a spent rate
    /// limit. Retrying with the same token will not help.
    #[error("{message}")]
    Unauthorized {
        /// The failure and every cause beneath it.
        message: String,
    },
    /// The rate limit is spent. Nothing will succeed until it resets.
    #[error("{message}")]
    RateLimited {
        /// The failure and every cause beneath it.
        message: String,
    },
    /// The repository is not `owner/name`, or does not exist, or this token
    /// cannot see it: GitHub answers the last two identically on purpose.
    #[error("{message}")]
    RepositoryUnavailable {
        /// The failure and every cause beneath it.
        message: String,
    },
    /// GitHub could not be reached, or answered with an HTTP status there is no
    /// other reading for. The one worth trying again.
    #[error("{message}")]
    Network {
        /// The failure and every cause beneath it.
        message: String,
    },
    /// GitHub answered, and the answer could not be read or represented.
    #[error("{message}")]
    Unreadable {
        /// The failure and every cause beneath it.
        message: String,
    },
}

impl FetchError {
    /// A transport failure, grouped by [`grouped`] and worded by its whole
    /// chain.
    fn from_github(error: &GitHubError) -> Self {
        grouped(error, chain(error))
    }

    /// A page that arrived and holds a value the domain cannot represent.
    ///
    /// Every [`ConversionError`] lands here, so this does not match on it:
    /// whichever value was refused, fetching again fetches the same one.
    fn from_conversion(error: &ConversionError) -> Self {
        Self::Unreadable {
            message: chain(error),
        }
    }

    /// A repository name that is not `owner/name`, refused before any request.
    fn from_repo(error: &InvalidValue) -> Self {
        Self::RepositoryUnavailable {
            message: chain(error),
        }
    }
}

/// Fetches `repo`'s board with the token in `GH_TOKEN` or `GITHUB_TOKEN`.
///
/// # Errors
///
/// [`FetchError::NoToken`] when there is no usable token, and otherwise as
/// [`fetch_board_with`].
#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_board(repo: String) -> Result<PopoverBoard, FetchError> {
    let client = GitHubClient::from_env().map_err(|error| FetchError::from_github(&error))?;
    fetch_board_with(&client, &repo).await
}

/// Fetches `repo`'s board with `client`.
///
/// Not exported over `UniFFI`: Swift never holds a client. Public so that tests
/// can point one at a local server.
///
/// # Errors
///
/// [`FetchError::RepositoryUnavailable`] when `repo` is not `owner/name`,
/// before any request is sent, or GitHub will not show it;
/// [`FetchError::Unauthorized`] or [`FetchError::RateLimited`] when GitHub
/// refuses; [`FetchError::Network`] when it cannot be reached or fails; and
/// [`FetchError::Unreadable`] when what it sent cannot be read.
pub async fn fetch_board_with(
    client: &GitHubClient,
    repo: &str,
) -> Result<PopoverBoard, FetchError> {
    let repo = RepoRef::parse(repo).map_err(|error| FetchError::from_repo(&error))?;
    let open = client
        .fetch_open_pull_requests(&repo)
        .await
        .map_err(|error| FetchError::from_github(&error))?;
    let judged = RepositoryFacts::try_from(open)
        .map_err(|error| FetchError::from_conversion(&error))?
        .judge();
    Ok(project(judged.board(), judged.viewer()))
}

/// Every section of `board`, and the core's counts, as Swift reads them.
fn project(board: &Board, viewer: &Login) -> PopoverBoard {
    PopoverBoard {
        viewer: viewer.as_str().to_owned(),
        needs_your_review: section(board.needs_your_review()),
        unclaimed: section(board.unclaimed()),
        you_reviewed: section(board.you_reviewed()),
        yours: section(board.yours()),
        your_drafts: section(board.your_drafts()),
        other_drafts: section(board.other_drafts()),
        reviewable_now: count(board.reviewable_now()),
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
        rereview: placed.rereview(),
        depth: count(placed.depth()),
    }
}

/// A count of pull requests, or a depth in a stack of them, as Swift reads it.
///
/// Saturating rather than fallible, because overflow is unreachable and a
/// guard that cannot fire would claim a protection that is not there. Both
/// numbers are bounded by the rows on the board: a depth is less than the rows
/// in its tree, and [`Board::total`] and [`Board::reviewable_now`] are at most
/// the rows handed in. Through [`fetch_board_with`] those rows are one fetch's
/// pull requests: at most 100 pages, each at GitHub's 100-per-page maximum.
/// The client caps the pages; GitHub caps the rows on each. That is under
/// `u32::MAX` by five orders of magnitude.
fn count(rows: usize) -> u32 {
    u32::try_from(rows).unwrap_or(u32::MAX)
}

/// `error` and every cause beneath it, as one line.
fn chain(error: &(dyn Error + 'static)) -> String {
    std::iter::successors(Some(error), |&error| error.source())
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ")
}

/// The variant `error` belongs in, carrying `message`.
///
/// Classified by what the last attempt failed with: a
/// [`GitHubError::GaveUp`] is looked through, however deeply nested, because
/// it says how long the client tried and not what went wrong. The app waits
/// out a spent rate limit differently from a flaky network, so a rate limit
/// the client gave up on must still read as one.
///
/// # The twin of `prtrack-cli`'s `exit.rs`
///
/// `Exit::of` in `crates/prtrack-cli/src/exit.rs` makes the same split for
/// the CLI's exit codes, and this crate cannot depend on that one, so the
/// mapping is restated here. They must agree wherever they overlap:
/// `NoToken` and `Unauthorized` together are its `Auth`;
/// `RepositoryUnavailable` is its `Usage`, `UnusableEndpoint` included; and
/// `RateLimited`, `Network` and `Unreadable` split its `Api`. One difference
/// is deliberate: `Exit::of` files every `GaveUp` under `Api` without looking
/// inside. The two still agree today, because the client only gives up on
/// failures it retried, and every one of those is `Api` there.
///
/// # A new variant does not fail this build
///
/// [`GitHubError`] is `#[non_exhaustive]` and defined in another crate, so
/// the language requires a wildcard here, and the lint that would flag a
/// variant it swallows (`non_exhaustive_omitted_patterns`) is unstable. Every
/// variant this build knows is named, and the wildcard shares the arm with
/// `Network` for the reason `exit.rs` gives: a retry wastes a request, where
/// any other reading sends somebody to fix something that is fine.
fn grouped(error: &GitHubError, message: String) -> FetchError {
    match error {
        GitHubError::GaveUp { source, .. } => grouped(source, message),
        GitHubError::MissingToken | GitHubError::UnusableToken => FetchError::NoToken { message },
        GitHubError::Unauthorized | GitHubError::Forbidden => FetchError::Unauthorized { message },
        GitHubError::RateLimited => FetchError::RateLimited { message },
        GitHubError::RepositoryUnavailable { .. } | GitHubError::UnusableEndpoint => {
            FetchError::RepositoryUnavailable { message }
        }
        GitHubError::Decode(_)
        | GitHubError::Malformed(_)
        | GitHubError::GraphQl { .. }
        | GitHubError::TooManyPages { .. } => FetchError::Unreadable { message },
        GitHubError::Transport(_) | GitHubError::UnexpectedStatus { .. } | _ => {
            FetchError::Network { message }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{grouped, project, FetchError, PopoverRow};
    use chrono::{DateTime, Utc};
    use prtrack_core::{
        Board, BoardRow, BranchName, Check, CheckName, CheckState, DraftState, HeadRepository,
        HeadSha, InvalidValue, Login, MergeState, MergeableState, Placed, PolicyRules, PrFacts,
        PrNumber, PrTitle, PrUrl, ReasonKind, RepoRef, Review, ReviewDecision, Reviewer, Verdict,
    };
    use prtrack_github::{ConversionError, GitHubError, Malformed};

    /// `source` wrapped in one [`GitHubError::GaveUp`].
    fn gave_up(source: GitHubError) -> GitHubError {
        GitHubError::GaveUp {
            attempts: 3,
            source: Box::new(source),
        }
    }

    /// Every [`GitHubError`] variant but `Transport`, beside the variant the
    /// app is handed for it.
    ///
    /// `Transport` carries a `reqwest::Error`, which nothing outside `reqwest`
    /// can build; `tests/fetch_board.rs` reaches it through a refused
    /// connection instead.
    fn every_github_error() -> Result<Vec<(GitHubError, FetchError)>, Box<dyn std::error::Error>> {
        let empty = String::new;
        let decode = serde_json::from_str::<serde_json::Value>("{")
            .err()
            .ok_or("an unclosed object is not JSON")?;
        Ok(vec![
            (
                GitHubError::MissingToken,
                FetchError::NoToken { message: empty() },
            ),
            (
                GitHubError::UnusableToken,
                FetchError::NoToken { message: empty() },
            ),
            (
                GitHubError::Unauthorized,
                FetchError::Unauthorized { message: empty() },
            ),
            (
                GitHubError::Forbidden,
                FetchError::Unauthorized { message: empty() },
            ),
            (
                GitHubError::RateLimited,
                FetchError::RateLimited { message: empty() },
            ),
            (
                GitHubError::RepositoryUnavailable {
                    repo: RepoRef::parse("example-org/example-repo")?,
                },
                FetchError::RepositoryUnavailable { message: empty() },
            ),
            // `exit.rs` files this beside an unseen repository, as a usage
            // error: something the person at the keyboard must change.
            (
                GitHubError::UnusableEndpoint,
                FetchError::RepositoryUnavailable { message: empty() },
            ),
            (
                GitHubError::UnexpectedStatus { status: 418 },
                FetchError::Network { message: empty() },
            ),
            (
                GitHubError::Decode(decode),
                FetchError::Unreadable { message: empty() },
            ),
            (
                GitHubError::Malformed(Malformed::NoPageInfo),
                FetchError::Unreadable { message: empty() },
            ),
            (
                GitHubError::GraphQl {
                    messages: vec!["Field 'nope' doesn't exist".to_owned()],
                },
                FetchError::Unreadable { message: empty() },
            ),
            (
                GitHubError::TooManyPages { pages: 101 },
                FetchError::Unreadable { message: empty() },
            ),
            // `GaveUp` says how long the client tried, not what failed: the
            // app is told what the last attempt failed with.
            (
                gave_up(GitHubError::RateLimited),
                FetchError::RateLimited { message: empty() },
            ),
            (
                gave_up(gave_up(GitHubError::RateLimited)),
                FetchError::RateLimited { message: empty() },
            ),
            (
                gave_up(GitHubError::UnexpectedStatus { status: 502 }),
                FetchError::Network { message: empty() },
            ),
            (
                gave_up(GitHubError::Malformed(Malformed::NoNodes)),
                FetchError::Unreadable { message: empty() },
            ),
        ])
    }

    #[test]
    fn each_github_failure_is_grouped_by_what_the_reader_should_do(
    ) -> Result<(), Box<dyn std::error::Error>> {
        for (error, expected) in every_github_error()? {
            assert_eq!(grouped(&error, String::new()), expected, "{error:?}");
        }
        Ok(())
    }

    #[test]
    fn a_failure_given_up_on_keeps_the_whole_chain_in_its_message() {
        let error = FetchError::from_github(&gave_up(GitHubError::RateLimited));

        // Classified by the root, worded by the whole chain: the message
        // still says the client gave up, and why.
        assert_eq!(
            error,
            FetchError::RateLimited {
                message: format!(
                    "{}: {}",
                    gave_up(GitHubError::RateLimited),
                    GitHubError::RateLimited
                ),
            }
        );
    }

    #[test]
    fn an_answer_that_cannot_be_represented_is_unreadable() {
        let error = FetchError::from_conversion(&ConversionError::NoCommit { number: 7 });

        assert_eq!(
            error,
            FetchError::Unreadable {
                message: ConversionError::NoCommit { number: 7 }.to_string(),
            }
        );
    }

    #[test]
    fn a_repository_that_is_not_owner_slash_name_is_unavailable() -> Result<(), &'static str> {
        let refused = RepoRef::parse("example-repo")
            .err()
            .ok_or("a name with no owner is refused")?;

        let error = FetchError::from_repo(&refused);

        assert_eq!(
            error,
            FetchError::RepositoryUnavailable {
                message: refused.to_string(),
            }
        );
        Ok(())
    }

    /// The commit the viewer reviewed: no longer the head.
    const EARLIER: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    /// The head the author pushed afterwards.
    const PUSHED: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    /// `author`'s open, green pull request from `head_ref` in this repository
    /// into `base_ref`, with nobody asked and nothing reviewed, at [`PUSHED`].
    ///
    /// The head is [`HeadRepository::ThisOne`] so that another pull request
    /// aimed at `head_ref` stacks on it; the recorded fixtures cannot, because
    /// they carry no `isCrossRepository` and every head there is unstated.
    fn on_branch(
        number: u32,
        author: &str,
        head_ref: &str,
        base_ref: &str,
    ) -> Result<PrFacts, InvalidValue> {
        Ok(PrFacts {
            repo: RepoRef::parse("example-org/example-repo")?,
            number: PrNumber::new(number)?,
            url: PrUrl::new(&format!(
                "https://github.com/example-org/example-repo/pull/{number}"
            )),
            title: PrTitle::new("Split the ledger writer"),
            author: Some(Login::parse(author)?),
            draft_state: DraftState::ReadyForReview,
            head: HeadSha::parse(PUSHED)?,
            head_ref: BranchName::new(head_ref),
            head_repository: HeadRepository::ThisOne,
            base_ref: BranchName::new(base_ref),
            review_decision: None,
            mergeable: MergeableState::Mergeable,
            merge_state: MergeState::Clean,
            requested_reviewers: Vec::new(),
            changes_requested_by: Vec::new(),
            reviews: Vec::new(),
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

    /// `monalisa`'s open pull request, green, on which `viewer` asked for
    /// changes at [`EARLIER`] before the author pushed [`PUSHED`].
    ///
    /// The change request is entered everywhere GitHub keeps it — the review,
    /// `changes_requested_by` and the review decision — because that stale fact
    /// is exactly what the row's own reason still reports and the viewer's
    /// reading sets aside.
    fn pushed_past_your_change_request(viewer: &Login) -> Result<PrFacts, InvalidValue> {
        let mut facts = on_branch(101, "monalisa", "topic/ledger", "main")?;
        facts.review_decision = Some(ReviewDecision::ChangesRequested);
        facts.changes_requested_by = vec![viewer.clone()];
        facts.reviews = vec![Review {
            by: viewer.clone(),
            verdict: Verdict::ChangesRequested,
            on: Some(HeadSha::parse(EARLIER)?),
        }];
        Ok(facts)
    }

    /// A board of every pull request in `facts`, under no branch rule.
    fn board_of(facts: Vec<PrFacts>, viewer: &Login) -> Board {
        let rows = facts
            .into_iter()
            .map(|facts| BoardRow::reduce(facts, &PolicyRules::unprotected()))
            .collect();
        Board::assemble(rows, viewer)
    }

    /// Each projected row's number and depth, in the order it arrived.
    fn placed(rows: &[PopoverRow]) -> Vec<(u32, u32)> {
        rows.iter().map(|row| (row.number, row.depth)).collect()
    }

    #[test]
    fn a_re_review_is_projected_with_the_reason_the_viewer_reads() -> Result<(), InvalidValue> {
        let viewer = Login::parse("octocat")?;
        let board = board_of(vec![pushed_past_your_change_request(&viewer)?], &viewer);
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
            .map(|row| (row.number, row.reason, row.rereview))
            .collect();
        assert_eq!(
            projected,
            Vec::from_iter(viewers.map(|reason| (101, reason, Some(Verdict::ChangesRequested))))
        );
        Ok(())
    }

    #[test]
    fn an_approval_pushed_past_is_projected_as_an_approval() -> Result<(), InvalidValue> {
        let viewer = Login::parse("octocat")?;
        let mut facts = on_branch(102, "monalisa", "topic/ledger", "main")?;
        facts.review_decision = Some(ReviewDecision::Approved);
        facts.reviews = vec![Review {
            by: viewer.clone(),
            verdict: Verdict::Approved,
            on: Some(HeadSha::parse(EARLIER)?),
        }];
        let board = board_of(vec![facts], &viewer);

        let popover = project(&board, &viewer);

        let projected: Vec<_> = popover
            .needs_your_review
            .iter()
            .map(|row| (row.number, row.rereview))
            .collect();
        assert_eq!(projected, [(102, Some(Verdict::Approved))]);
        Ok(())
    }

    #[test]
    fn each_row_carries_its_own_title_author_and_link() -> Result<(), InvalidValue> {
        let viewer = Login::parse("octocat")?;
        // Distinct titles, so a title taken from the wrong row or field shows.
        let mut named = on_branch(7, "monalisa", "topic/c1", "main")?;
        named.title = PrTitle::new("Retire the nightly export");
        // A deleted account leaves no author, which must stay absent.
        let mut orphaned = on_branch(8, "hubot", "topic/c2", "main")?;
        orphaned.title = PrTitle::new("Pin the toolchain");
        orphaned.author = None;
        let board = board_of(vec![named, orphaned], &viewer);

        let popover = project(&board, &viewer);

        let mut cells: Vec<_> = [
            &popover.needs_your_review,
            &popover.unclaimed,
            &popover.you_reviewed,
            &popover.yours,
            &popover.your_drafts,
            &popover.other_drafts,
        ]
        .into_iter()
        .flatten()
        .map(|row| {
            (
                row.number,
                row.title.as_str(),
                row.author.as_deref(),
                row.url.as_str(),
            )
        })
        .collect();
        cells.sort_unstable();
        assert_eq!(
            cells,
            [
                (
                    7,
                    "Retire the nightly export",
                    Some("monalisa"),
                    "https://github.com/example-org/example-repo/pull/7"
                ),
                (
                    8,
                    "Pin the toolchain",
                    None,
                    "https://github.com/example-org/example-repo/pull/8"
                ),
            ]
        );
        Ok(())
    }

    #[test]
    fn a_stack_is_projected_whole_in_its_roots_section() -> Result<(), InvalidValue> {
        let viewer = Login::parse("octocat")?;
        // Three deep, so the walk's order and every depth past the first are
        // both observable.
        let root = on_branch(1, "octocat", "topic/a1", "main")?;
        let child = on_branch(2, "octocat", "topic/a2", "topic/a1")?;
        let grandchild = on_branch(3, "octocat", "topic/a3", "topic/a2")?;
        // Approved by the viewer at its head, so it is drawn in `you_reviewed`.
        let mut approved = on_branch(10, "monalisa", "topic/b1", "main")?;
        approved.reviews = vec![Review {
            by: viewer.clone(),
            verdict: Verdict::Approved,
            on: Some(HeadSha::parse(PUSHED)?),
        }];
        // Asked of the viewer, so on its own it would be in the review section;
        // stacked on the approved one, it is drawn there instead.
        let mut asked = on_branch(11, "hubot", "topic/b2", "topic/b1")?;
        asked.requested_reviewers = vec![Reviewer::User(viewer.clone())];
        let board = board_of(vec![grandchild, asked, root, approved, child], &viewer);

        let popover = project(&board, &viewer);

        assert_eq!(placed(&popover.yours), [(1, 0), (2, 1), (3, 2)]);
        assert_eq!(placed(&popover.you_reviewed), [(10, 0), (11, 1)]);
        assert_eq!(
            placed(&popover.needs_your_review),
            [],
            "the review request is drawn under its root, not on its own"
        );
        assert_eq!(
            popover.reviewable_now, 1,
            "the review request counts where it would sit alone"
        );
        let rereviews: Vec<_> = [&popover.yours, &popover.you_reviewed]
            .into_iter()
            .flatten()
            .map(|row| (row.number, row.rereview))
            .collect();
        assert_eq!(
            rereviews,
            [(1, None), (2, None), (3, None), (10, None), (11, None)],
            "an approval at the head, and no review at all, are not re-reviews"
        );
        Ok(())
    }
}
