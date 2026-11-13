//! `fetch_board_with` against a local server answering the recorded pages.
//!
//! The expected board is built from the same recording through the same public
//! path the CLI uses, never written out as literals, so a re-recorded fixture
//! cannot leave these assertions comparing against a board that no longer
//! exists.

#![forbid(unsafe_code)]

use std::num::NonZeroU32;
use std::time::Duration;

use prtrack_core::{RepoRef, Stacks};
use prtrack_ffi::{fetch_board_with, FetchError, PopoverRow};
use prtrack_github::{GitHubClient, RepositoryFacts, RetryBudget, Token};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// The first recorded page of open pull requests.
const PAGE_1: &str =
    include_str!("../../prtrack-github/tests/fixtures/open-pull-requests-page-1.json");

/// The second, and last, recorded page.
const PAGE_2: &str =
    include_str!("../../prtrack-github/tests/fixtures/open-pull-requests-page-2.json");

/// The repository the recordings claim to come from, after redaction.
const REPO: &str = "example-org/example-repo";

/// A token-shaped string no real account issued, checked for by substring in
/// the error text.
const SECRET: &str = "ghp_0123456789abcdefSECRETzzzz";

/// What a page request carries when it asks for the first page. Compact, as
/// `reqwest` serialises it.
const FIRST_PAGE: &[u8] = br#""after":null"#;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Whether `request` asks for the first page rather than one after a cursor.
fn asks_for_the_first_page(request: &Request) -> bool {
    request
        .body
        .windows(FIRST_PAGE.len())
        .any(|window| window == FIRST_PAGE)
}

/// A 200 carrying `body` as JSON, the way GitHub's GraphQL endpoint answers.
fn ok(body: &str) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body.to_owned(), "application/json")
}

/// A server answering each page request with the recorded page it asks for.
///
/// Chosen by the request's cursor rather than by arrival order, so the test
/// can fetch the same repository twice — once for the expected board, once
/// through the code under test — without scripting the sequence.
async fn recorded_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(asks_for_the_first_page)
        .respond_with(ok(PAGE_1))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(|request: &Request| !asks_for_the_first_page(request))
        .respond_with(ok(PAGE_2))
        .mount(&server)
        .await;
    server
}

/// A client pointed at `server`, trying once and never sleeping, so a test of
/// a failure does not sit through real backoff.
fn client_for(server: &MockServer) -> Result<GitHubClient, Box<dyn std::error::Error>> {
    Ok(GitHubClient::new(&Token::parse(SECRET)?)?
        .with_endpoint(&format!("{}/graphql", server.uri()))?
        .with_retry_budget(RetryBudget::new(NonZeroU32::MIN, Duration::ZERO)))
}

/// The pull request numbers of a section, in drawing order.
fn walked(section: &Stacks) -> Vec<u32> {
    section
        .walk()
        .iter()
        .map(|placed| placed.row().facts().number.get())
        .collect()
}

/// The pull request numbers of projected rows, in the order they arrived.
fn projected(rows: &[PopoverRow]) -> Vec<u32> {
    rows.iter().map(|row| row.number).collect()
}

#[tokio::test]
async fn both_sections_arrive_in_the_order_the_core_drew_them() -> TestResult {
    let server = recorded_server().await;
    let client = client_for(&server)?;

    let open = client
        .fetch_open_pull_requests(&RepoRef::parse(REPO)?)
        .await?;
    let judged = RepositoryFacts::try_from(open)?.judge();
    let expected_review = walked(judged.board().needs_your_review());
    let expected_yours = walked(judged.board().yours());
    // An empty section matches any order, so the comparison below would pass
    // against a projection that drops or shuffles everything.
    assert!(
        !expected_review.is_empty(),
        "the recording has rows to review"
    );
    assert!(
        !expected_yours.is_empty(),
        "the recording has the viewer's own rows"
    );

    let popover = fetch_board_with(&client, REPO).await?;

    assert_eq!(projected(&popover.needs_your_review), expected_review);
    assert_eq!(projected(&popover.yours), expected_yours);
    assert_eq!(popover.viewer, judged.viewer().as_str());
    assert_eq!(
        usize::try_from(popover.total).ok(),
        Some(judged.board().total())
    );
    Ok(())
}

#[tokio::test]
async fn a_server_error_is_a_failure_and_never_an_empty_board() -> TestResult {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let client = client_for(&server)?;

    let fetched = fetch_board_with(&client, REPO).await;

    assert!(
        matches!(&fetched, Err(FetchError::Failed { .. })),
        "a 500 must be a failure, got {fetched:?}"
    );
    let message = fetched.err().map(|error| error.to_string());
    assert!(
        message
            .as_deref()
            .is_some_and(|text| !text.is_empty() && !text.contains(SECRET)),
        "a failure says what failed, without the token: {message:?}"
    );
    Ok(())
}
