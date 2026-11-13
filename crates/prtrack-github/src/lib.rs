//! Fetching open pull requests from GitHub's GraphQL API.
//!
//! This crate is the transport layer and nothing else. It sends one query, page
//! by page, and hands back what came off the wire.
//!
//! # Two layers, and the boundary between them
//!
//! [`OpenPullRequests`] is what came off the wire. [`RepositoryFacts`] is the
//! same response converted into `prtrack-core`'s domain types, and every value
//! in it has had its invariants checked. The conversion happens once, at
//! `TryFrom`, so no consumer re-validates and none can forget to. The wire
//! types in between are private to this crate, so nothing downstream can depend
//! on GitHub's shape instead of the domain's.
//!
//! Each pull request comes back beside the `PolicyRules` that judge *it*,
//! rather than one policy for the board. That is not a convenience: a stacked
//! pull request aimed at another feature branch is governed by no rule even
//! where its neighbours are governed by a strict one, and judging it against
//! the strict one would sort its failing checks into blocking and advisory by a
//! list it is not subject to.
//!
//! # What it does take care of
//!
//! - **The preview media type.** `mergeStateStatus` does not resolve without
//!   `application/vnd.github.merge-info-preview+json`, so every request carries
//!   it.
//! - **Pagination.** The whole list, followed across pages, with a cap so that
//!   a server which keeps promising another page cannot spin a poll loop
//!   forever.
//! - **Retrying what is worth retrying.** Measured against the live API, about
//!   one page in nine came back unusable and the next attempt was fine. A page
//!   with a hole in it is refused and retried rather than accepted, because a
//!   silently short list reads as a repository that merges faster than it does.
//!   A rejected token, an invisible repository, and a spent rate limit are not
//!   retried, because asking again cannot change any of them.
//! - **A timeout on every attempt**, so a hung connection costs one timeout
//!   rather than the whole poll.
//! - **Keeping the credential out of everything printable.** The token is read
//!   from the environment, never from a command line — `argv` is world-readable
//!   through `ps` — and it is held only as a header value marked sensitive.
//!   `Debug` is written by hand wherever one could otherwise reach a log, and
//!   no error message in [`GitHubError`] contains it.
//!
//! # Failure is never an empty result
//!
//! An `Ok` holding nothing means the repository has no open pull requests.
//! Every way of failing to find that out is an `Err`. The two must stay
//! distinguishable: an empty board that actually means "GitHub is down" is a
//! lie to the person reading it.
//!
//! # Examples
//!
//! ```no_run
//! use prtrack_core::{BoardRow, RepoRef};
//! use prtrack_github::{GitHubClient, RepositoryFacts};
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let client = GitHubClient::from_env()?;
//! let open = client
//!     .fetch_open_pull_requests(&RepoRef::parse("octocat/hello-world")?)
//!     .await?;
//! eprintln!("{} open pull requests", open.collected());
//!
//! let facts = RepositoryFacts::try_from(open)?;
//! for (pull_request, policy) in facts.pull_requests() {
//!     let _row = BoardRow::reduce(pull_request.clone(), policy);
//! }
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

mod client;
mod error;
mod facts;
mod pattern;
mod policy;
mod retry;
mod token;
mod wire;

pub use client::{Asset, GitHubClient, OpenPullRequests, Release};
pub use error::{ConversionError, GitHubError, Malformed, NestedList};
pub use facts::RepositoryFacts;
pub use retry::RetryBudget;
pub use token::{Token, GH_TOKEN, GITHUB_TOKEN};
