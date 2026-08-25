//! The prtrack decision core: what is true about a pull request, what the
//! repository's policy requires of it, and therefore why it cannot merge yet.
//!
//! # This crate performs no IO
//!
//! Nothing here opens a socket, reads a file, reads an environment variable, or
//! reads the clock. Every function that needs the current time takes `now` as a
//! parameter, so a caller can replay history or drive a test from a fixed
//! instant without a controlled-clock abstraction.
//!
//! That restriction is what lets the same decisions run behind a terminal, a
//! macOS menubar app, and a hosted backend without any of them disagreeing. It
//! also means the crate depends on no other crate in this workspace: facts
//! arrive already parsed, and the caller owns how they were fetched.
//!
//! # Where to start
//!
//! The vocabulary comes first. [`RepoRef`], [`PrNumber`], [`Login`],
//! [`CheckName`], and [`HeadSha`] are checked newtypes over the primitives
//! GitHub returns. Each has private fields and a constructor that validates, so
//! a value that reached this crate cannot be malformed, and two values that
//! mean different things cannot be swapped at a call site — a login will not
//! compile where a check name is wanted, however alike they look on the wire.
//!
//! All of them report construction failure as [`InvalidValue`], so a caller
//! converting a whole payload handles one error type.
//!
//! [`PolicyRules`] is what a pull request is judged against: the repository's
//! own branch protection rule, parsed into the questions the board asks of it.
//! Read [`BlockingChecks`] before using it. Which failures can hold a merge
//! back is the repository's decision, not ours — most checks a pull request
//! runs are advisory and cannot block anything — and a policy that names no
//! required checks means "we cannot tell", which falls back to treating every
//! failure as blocking. That fallback is answered inside the type, by
//! [`PolicyRules::blocks_merge_when_failing`].
//!
//! [`classify`] is where the two meet: it takes the [`Check`] attempts a pull
//! request reported and returns a [`Classified`] splitting them into blocking,
//! advisory, running, and passing. Two things about it surprise people. There
//! is no "is this pull request governed" flag — a pull request no rule covers
//! is classified against [`PolicyRules::unprotected`], so the fallback lives in
//! one place. And the input is every *attempt*, not every check: GitHub leaves
//! a re-run's predecessor in the rollup, so each name is reduced to its newest
//! attempt before anything is decided.

#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]

mod board;
mod checks;
mod decision;
mod facts;
mod nudge;
mod policy;
mod types;

pub use board::{ActionUrgency, Board, BoardRow, Reviewability};
pub use checks::{classify, Check, CheckState, Classified};
pub use decision::{BlockedReason, FailingCheckCount, NonEmpty, UnresolvedThreadCount};
pub use facts::{
    BranchName, DraftState, MergeState, MergeableState, PrFacts, PrTitle, PrUrl, ReviewDecision,
    Reviewer, TeamSlug,
};
pub use nudge::{
    observe, ApprovalState, Nudge, Observation, ReasonKind, RequiredCheckHealth, ReviewRequest,
    Watched,
};
pub use policy::{BlockingChecks, PolicyRules, RequiredChecks, Requirement, StaleReviewHandling};
pub use types::{CheckName, HeadSha, InvalidValue, Login, PrNumber, RepoRef};
