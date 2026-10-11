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
pub async fn poll(
    client: &GitHubClient,
    repo: &RepoRef,
    check: Option<&UpdateCheck>,
    now: i64,
) -> Poll {
    let read = client.fetch_open_pull_requests(repo).await;
    let update = match check {
        Some(check) => check.run(client, now).await,
        None => Update::Unknown,
    };
    Poll { read, update }
}

#[cfg(test)]
mod tests {
    use super::poll;
    use crate::update::{Update, UpdateCheck};
    use prtrack_core::RepoRef;
    use prtrack_github::{GitHubClient, RetryBudget, Token};
    use std::num::NonZeroU32;
    use std::time::Duration;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// A token-shaped string no real account issued.
    const TOKEN: &str = "ghp_0123456789abcdefNOTAREALTOKEN";

    #[tokio::test]
    async fn a_read_that_fails_still_says_a_newer_release_is_out() -> anyhow::Result<()> {
        // The case this module exists for: GitHub refuses every page, and the
        // release that fixes it is already published. Saying nothing here
        // leaves somebody watching the same error on every run with the cure
        // one command away.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/example-org/prtrack/releases/latest"))
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

        assert!(
            polled.read.is_err(),
            "the read was refused: {:?}",
            polled.read
        );
        assert_eq!(
            polled.update,
            Update::Newer {
                version: "999.0.0".to_owned()
            }
        );
        Ok(())
    }
}
