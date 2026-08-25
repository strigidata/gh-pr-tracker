//! Why a fetch did not produce a response this crate could read.

use prtrack_core::RepoRef;

use crate::token::{GH_TOKEN, GITHUB_TOKEN};

/// Why a call to the GitHub API did not yield a readable response.
///
/// Every variant is a distinct thing a shell might do something different
/// about: an exhausted rate limit is worth waiting out, a rejected token is
/// not, and a malformed page is worth retrying but an unreadable query is not.
/// That is why they are separate variants rather than one string.
///
/// # Nothing here leaks a credential
///
/// No `Display` text in this enum contains the token, a URL carrying
/// credentials, or a path from the machine it ran on. The token is never
/// interpolated anywhere — [`GitHubError::UnusableToken`] names the problem
/// without echoing the value — and an endpoint carrying a username or password
/// is rejected at construction, so it cannot reach the [`reqwest::Error`] that
/// [`GitHubError::Transport`] carries as its source. A test in
/// the client holds the whole chain against a known token to keep that
/// true.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GitHubError {
    /// Neither environment variable named a token.
    #[error("no GitHub token in the environment: set {GH_TOKEN} or {GITHUB_TOKEN}")]
    MissingToken,

    /// A token was supplied that cannot be sent as an HTTP credential.
    ///
    /// The offending value is deliberately absent from the message: it is a
    /// credential, and a message ends up in logs.
    #[error("the GitHub token contains characters an HTTP header cannot carry")]
    UnusableToken,

    /// The endpoint was not an absolute `http`/`https` URL, or it carried a
    /// username or password.
    ///
    /// Credentials in the URL are refused rather than stripped, because a URL
    /// travels into [`reqwest::Error`]'s own message and out to a log from
    /// there. The URL is not echoed here for the same reason.
    #[error("the API endpoint must be an http or https URL with no credentials in it")]
    UnusableEndpoint,

    /// The request could not be sent, or no answer arrived before the timeout.
    #[error("the request to GitHub could not be completed")]
    Transport(#[source] reqwest::Error),

    /// GitHub rejected the credential — HTTP 401.
    #[error("GitHub rejected the token")]
    Unauthorized,

    /// GitHub refused the request for a reason other than an exhausted rate
    /// limit — HTTP 403.
    #[error("GitHub refused the request")]
    Forbidden,

    /// The rate limit is spent, so no further request will succeed until it
    /// resets.
    #[error("the GitHub API rate limit is exhausted")]
    RateLimited,

    /// GitHub answered with a status this crate has no reading for.
    #[error("GitHub answered with HTTP {status}")]
    UnexpectedStatus {
        /// The status code, as GitHub sent it.
        status: u16,
    },

    /// The body was not the JSON envelope this query asks for.
    #[error("GitHub's answer could not be read as the response to this query")]
    Decode(#[source] serde_json::Error),

    /// The envelope parsed, but a part the caller depends on was missing.
    #[error(transparent)]
    Malformed(#[from] Malformed),

    /// GitHub answered with errors and no data.
    ///
    /// The messages are GitHub's own. They can name fields of the query, which
    /// is the point of showing them.
    #[error("GitHub rejected the query: {}", .messages.join("; "))]
    GraphQl {
        /// Every `message` GitHub returned, in the order it returned them.
        messages: Vec<String>,
    },

    /// The response carried no repository, which is how GitHub reports one that
    /// does not exist or that this token cannot see.
    ///
    /// The two cases are deliberately not distinguished: GitHub reports a
    /// private repository as absent precisely so that a token cannot be used to
    /// probe for one, and inventing a distinction here would be a guess.
    #[error("no repository {repo} is visible to this token")]
    RepositoryUnavailable {
        /// The repository that was asked for.
        repo: RepoRef,
    },

    /// A failure that looked transient did not clear within the retry budget.
    ///
    /// The last failure is the source, so the chain still says what actually
    /// went wrong.
    #[error("gave up after {attempts} attempt(s)")]
    GaveUp {
        /// How many attempts were made, including the first.
        attempts: u32,
        /// What the final attempt failed with.
        #[source]
        source: Box<Self>,
    },

    /// Pagination ran past its cap without GitHub saying the list had ended.
    ///
    /// A server that keeps answering "there is another page" would otherwise
    /// spin forever against a poll loop.
    #[error("stopped after {pages} pages without reaching the end of the list")]
    TooManyPages {
        /// How many pages were fetched before stopping.
        pages: usize,
    },
}

/// A connection nested inside a pull request that this crate reads whole.
///
/// Named rather than described, so that a short list says which list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NestedList {
    /// The repository's branch protection rules.
    ProtectionRules,
    /// Who is still being waited on for a review.
    ReviewRequests,
    /// The latest review from each reviewer.
    LatestReviews,
    /// The review threads on the diff.
    ReviewThreads,
    /// Every attempt reported on the head commit.
    CheckContexts,
}

impl std::fmt::Display for NestedList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ProtectionRules => "branch protection rules",
            Self::ReviewRequests => "review requests",
            Self::LatestReviews => "latest reviews",
            Self::ReviewThreads => "review threads",
            Self::CheckContexts => "check contexts",
        })
    }
}

/// A response that parsed but was missing something the caller depends on.
///
/// These are separated from [`GitHubError::Decode`] because they are *not* a
/// bad query: they are a well-formed envelope with a hole in it, which is what
/// GitHub's occasional flaky page looks like. The client treats them as worth
/// retrying, and only reports one after the budget is spent.
///
/// Every variant here describes a way the population could silently shrink. A
/// short list that looks complete is the failure this crate exists to avoid, so
/// none of these is repaired or skipped past.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Malformed {
    /// The envelope carried neither `data` nor an explanation of its absence.
    #[error("GitHub's answer carried no data and no explanation")]
    NoData,

    /// A page arrived without `pageInfo`.
    ///
    /// Treating that as "the last page" would turn a truncated list into a
    /// complete-looking one.
    #[error(
        "a page of pull requests carried no pageInfo, so there is no telling whether more follow"
    )]
    NoPageInfo,

    /// A page said another follows but named no cursor to fetch it with.
    #[error("a page claimed another follows but named no cursor to fetch it with")]
    NoCursor,

    /// A page arrived without its `nodes` list, which is not the same as an
    /// empty one.
    #[error("a page of pull requests carried no nodes list")]
    NoNodes,

    /// The pages did not add up to the total GitHub reported.
    ///
    /// `hasNextPage: false` says the cursor reached the end, not that
    /// everything was seen. A pull request created between two requests sorts
    /// ahead of the cursor under any ordering and is stepped over, and GitHub
    /// reports the list as ended regardless. The arithmetic is the only signal
    /// there is, and a short board is indistinguishable from a small
    /// repository once it has been rendered.
    #[error(
        "the pages did not add up: {collected} pull requests, against a total \
         GitHub reported as {first_total} and then {last_total}"
    )]
    ListMoved {
        /// How many pull requests came back across every page.
        collected: usize,
        /// The total the first page reported.
        first_total: i64,
        /// The total the last page reported.
        last_total: i64,
    },

    /// A field was nulled by an error and the rest of the response returned
    /// around it.
    ///
    /// This is GraphQL working as designed: an error nulls the field it
    /// happened at and the query keeps going. The trouble is that the result
    /// is indistinguishable from the field being legitimately empty. A
    /// `statusCheckRollup` nulled by a timeout reads exactly like a pull
    /// request whose CI never ran, and that renders as a green board rather
    /// than as a failure.
    ///
    /// The path is the only thing that separates them, which is why it is
    /// captured.
    #[error("the response nulled `{path}` and returned the rest around it")]
    ErrorNulledField {
        /// Where the failure happened, as GitHub reported it.
        path: String,
    },

    /// A union member this crate names arrived in the fallback arm, which means
    /// its own fields did not parse.
    ///
    /// The fallback exists so that a member GitHub adds does not cost the page.
    /// It is untagged, so it also swallows a *known* member whose shape
    /// changed — a `CheckRun` whose `name` was renamed arrives as an unknown
    /// node calling itself a `CheckRun`.
    ///
    /// That is worse than a parse error, because it is quiet. Every check run
    /// would become a context with no readable state, every such state
    /// classifies as still running, and a repository with failing required
    /// checks would render as one waiting for CI.
    #[error("a `{typename}` did not parse and fell through to the unknown arm")]
    UnreadableMember {
        /// The type name it claimed, which is one this crate knows.
        typename: String,
    },

    /// A nested connection returned fewer entries than it says exist.
    ///
    /// Every one of them is capped by the query, and the caps are not
    /// generous: check contexts peaked at 76 of 100 on the recorded
    /// repository. Going over a cap is silent, and the consequences are
    /// asymmetric and all bad — a dropped attempt at a required check is a
    /// false green, a dropped review loses a `CHANGES_REQUESTED`, and a missed
    /// protection rule falls back to treating every failure as blocking.
    #[error("the {what} came back with {counted} of {total} entries")]
    Truncated {
        /// Which connection was short.
        what: NestedList,
        /// How many entries came back.
        counted: usize,
        /// How many it says exist.
        total: i64,
    },

    /// A nested connection reported no total, so there is no telling whether
    /// it is complete.
    ///
    /// Refused rather than assumed whole, for the same reason a page with no
    /// total is: an absent number is not agreement.
    #[error("the {what} reported no total, so there is no telling what is missing")]
    Uncountable {
        /// Which connection could not be checked.
        what: NestedList,
    },

    /// A page carried a null entry where a pull request should have been.
    #[error("a page of pull requests carried a null node, so the list is short by at least one")]
    NullNode,
}

/// Why a fetched response could not be turned into domain facts.
///
/// Separate from [`GitHubError`] because it is a different kind of failure: the
/// request succeeded and the response parsed, and then a value inside it turned
/// out not to be one the domain can represent. A caller can retry a
/// [`GitHubError`]; retrying this would fetch the same value again.
///
/// Every variant names the pull request or the rule it came from. A conversion
/// error that says only "invalid login" leaves the reader to find which of
/// forty pull requests it meant.
///
/// # This fails the whole conversion, not one row
///
/// A value GitHub sent that this crate cannot read is not a pull request to
/// skip. Skipping one would shorten the board silently, which reads as a
/// repository with less open work than it has — the failure this crate exists
/// to prevent. In practice none of these can fire against the live API; if one
/// ever does, it should be loud.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConversionError {
    /// The token's own account name is not a login this crate can read.
    #[error("the account this token authenticates as is not a login this crate can read")]
    Viewer {
        /// Why it was refused.
        #[source]
        source: prtrack_core::InvalidValue,
    },

    /// A pull request carried a value no domain type accepts.
    #[error("pull request {number} carried a value this crate cannot represent")]
    PullRequest {
        /// The pull request's number, as GitHub reported it.
        number: i64,
        /// Which value, and why it was refused.
        #[source]
        source: prtrack_core::InvalidValue,
    },

    /// A branch protection rule carried a value no domain type accepts.
    #[error(
        "the branch protection rule for {pattern:?} carried a value this crate cannot represent"
    )]
    ProtectionRule {
        /// The branch name pattern the rule protects.
        pattern: String,
        /// Which value, and why it was refused.
        #[source]
        source: prtrack_core::InvalidValue,
    },

    /// A rule demanded a number of approvals that is not a number of approvals.
    #[error("the branch protection rule for {pattern:?} demands {count} approvals")]
    ApprovalCount {
        /// The branch name pattern the rule protects.
        pattern: String,
        /// What it asked for.
        count: i64,
    },

    /// A pull request arrived with no commit.
    ///
    /// Every pull request has at least one, so this is a fetch that went wrong
    /// rather than a state to render. Its checks and its push time are both
    /// statements about a commit that is not there.
    #[error("pull request {number} carried no commit, so its checks describe nothing")]
    NoCommit {
        /// The pull request's number, as GitHub reported it.
        number: i64,
    },

    /// A timestamp GitHub sent is not an instant.
    ///
    /// Not repaired to `None`. An attempt with no instant loses the recency
    /// tie-break, so a timestamp quietly dropped could let a superseded failure
    /// speak for a check that has since been re-run.
    #[error("pull request {number} carried a timestamp that is not an instant")]
    Timestamp {
        /// The pull request's number, as GitHub reported it.
        number: i64,
        /// Why the instant could not be read.
        #[source]
        source: chrono::ParseError,
    },

    /// A count arrived negative, which no count can be.
    #[error("pull request {number} reported a count of {count}")]
    NegativeCount {
        /// The pull request's number, as GitHub reported it.
        number: i64,
        /// What it reported.
        count: i64,
    },
}
