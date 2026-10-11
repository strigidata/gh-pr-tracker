//! One read of a repository, and whether a newer `prtrack` is out.
//!
//! The two travel together because the run where the read fails is the run
//! where the version check matters most: a board that GitHub keeps refusing may
//! be refusing *this build*, and the release that fixes it is no use to anybody
//! who is never told it exists.

use prtrack_core::RepoRef;
use prtrack_github::{GitHubClient, GitHubError, OpenPullRequests};

use crate::update::{Update, UpdateCheck};

/// What one poll learned.
#[derive(Debug)]
pub struct Poll {
    /// The repository's open pull requests, or why they could not be read.
    pub read: Result<OpenPullRequests, GitHubError>,
    /// Whether a newer `prtrack` is out, asked whichever way the read went.
    pub update: Update,
}

/// Reads `repo`, then asks whether a newer `prtrack` is out.
///
/// The read comes first so that a board is never kept waiting on a courtesy.
/// The check runs whether or not the read succeeded; where GitHub is down
/// altogether it fails too, and an unanswered check says nothing. With no
/// check to run at all, the answer is [`Update::Unknown`].
///
/// The one exception is a read GitHub refused *the token* for: a spent rate
/// limit asks for quiet, and a rejected token cannot read releases either, so
/// asking would be a request made only to be refused.
pub async fn poll(
    client: &GitHubClient,
    repo: &RepoRef,
    check: Option<&UpdateCheck>,
    now: i64,
) -> Poll {
    let read = client.fetch_open_pull_requests(repo).await;
    let token_refused = matches!(
        read,
        Err(GitHubError::RateLimited | GitHubError::Unauthorized)
    );
    let update = match check {
        Some(check) if !token_refused => check.run(client, now).await,
        Some(_) | None => Update::Unknown,
    };
    Poll { read, update }
}

#[cfg(test)]
mod tests {
    use super::{poll, Poll};
    use crate::update::{Update, UpdateCheck};
    use prtrack_core::RepoRef;
    use prtrack_github::{GitHubClient, RetryBudget, Token};
    use std::num::NonZeroU32;
    use std::time::Duration;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// A token-shaped string no real account issued.
    const TOKEN: &str = "ghp_0123456789abcdefNOTAREALTOKEN";

    /// Where the test server publishes its releases.
    const RELEASES: &str = "/repos/example-org/prtrack/releases/latest";

    /// Polls a server that answers every read with `refusal` and has a newer
    /// release published, returning the poll and how many times the release
    /// was asked for.
    async fn poll_refused_with(refusal: ResponseTemplate) -> anyhow::Result<(Poll, usize)> {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(refusal)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(RELEASES))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(r#"{"tag_name":"v999.0.0"}"#, "application/json"),
            )
            .mount(&server)
            .await;

        let client = GitHubClient::new(&Token::parse(TOKEN)?)?
            .with_endpoint(&format!("{}/graphql", server.uri()))?
            .with_retry_budget(RetryBudget::new(NonZeroU32::MIN, Duration::ZERO));
        let check = UpdateCheck {
            release: RepoRef::parse("example-org/prtrack")?,
            cache: None,
        };

        let polled = poll(
            &client,
            &RepoRef::parse("example-org/example-repo")?,
            Some(&check),
            0,
        )
        .await;
        let asked = server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .filter(|request| request.url.path() == RELEASES)
            .count();
        Ok((polled, asked))
    }

    #[tokio::test]
    async fn a_read_that_fails_still_says_a_newer_release_is_out() -> anyhow::Result<()> {
        // The case this module exists for: GitHub refuses every page, and the
        // release that fixes it is already published. Saying nothing here
        // leaves somebody watching the same error on every run with the cure
        // one command away. A plain 403 is here too: the token cannot see this
        // repository, which says nothing about whether it can see releases.
        for status in [503_u16, 502, 403] {
            let (polled, _asked) = poll_refused_with(ResponseTemplate::new(status)).await?;

            assert!(polled.read.is_err(), "{status}: the read was refused");
            assert_eq!(
                polled.update,
                Update::Newer {
                    version: "999.0.0".to_owned()
                },
                "{status}"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn a_token_github_refuses_is_not_spent_on_the_version_check() -> anyhow::Result<()> {
        // A spent rate limit says "stop asking", and a rejected token cannot
        // read releases either. Asking anyway is a request made only to be
        // refused, in exactly the window GitHub asked for quiet.
        let cases = [
            (
                "a spent rate limit",
                ResponseTemplate::new(403).insert_header("x-ratelimit-remaining", "0"),
            ),
            ("a secondary rate limit", ResponseTemplate::new(429)),
            ("a rejected token", ResponseTemplate::new(401)),
        ];
        for (what, refusal) in cases {
            let (polled, asked) = poll_refused_with(refusal).await?;

            assert!(polled.read.is_err(), "{what}: the read was refused");
            assert_eq!(asked, 0, "{what}: the release must not be asked for");
            assert_eq!(polled.update, Update::Unknown, "{what}");
        }
        Ok(())
    }
}
