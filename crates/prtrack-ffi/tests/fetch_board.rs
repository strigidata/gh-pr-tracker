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

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The `variables.after` cursor a page request carries: `Null` when it asks
/// for the first page, whether GitHub's client sent `null` or left the key
/// out, and `None` when the body is not JSON at all, so such a request matches
/// neither page and the test fails rather than being handed one.
fn after(request: &Request) -> Option<serde_json::Value> {
    let body = serde_json::from_slice::<serde_json::Value>(&request.body).ok()?;
    Some(
        body.get("variables")
            .and_then(|variables| variables.get("after"))
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    )
}

/// Whether `request` asks for the first page.
fn asks_for_the_first_page(request: &Request) -> bool {
    after(request).is_some_and(|cursor| cursor.is_null())
}

/// Whether `request` asks for a page after a cursor.
fn asks_for_a_later_page(request: &Request) -> bool {
    after(request).is_some_and(|cursor| !cursor.is_null())
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
        .and(asks_for_a_later_page)
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
async fn every_section_arrives_in_the_order_the_core_drew_it() -> TestResult {
    let server = recorded_server().await;
    let client = client_for(&server)?;

    let open = client
        .fetch_open_pull_requests(&RepoRef::parse(REPO)?)
        .await?;
    let judged = RepositoryFacts::try_from(open)?.judge();
    let board = judged.board();
    // An empty section matches any order, so the comparison below would pass
    // against a projection that drops or shuffles everything. Two sections
    // the recording does fill are enough to rule that out; a section it
    // leaves empty is covered by the synthetic board in `lib.rs`.
    assert!(
        !walked(board.needs_your_review()).is_empty(),
        "the recording has rows to review"
    );
    assert!(
        !walked(board.yours()).is_empty(),
        "the recording has the viewer's own rows"
    );

    let popover = fetch_board_with(&client, REPO).await?;

    let sections = [
        (&popover.needs_your_review, board.needs_your_review()),
        (&popover.unclaimed, board.unclaimed()),
        (&popover.you_reviewed, board.you_reviewed()),
        (&popover.yours, board.yours()),
        (&popover.your_drafts, board.your_drafts()),
        (&popover.other_drafts, board.other_drafts()),
    ];
    for (rows, section) in sections {
        assert_eq!(projected(rows), walked(section));
    }
    assert_eq!(popover.viewer, judged.viewer().as_str());
    assert_eq!(
        usize::try_from(popover.reviewable_now).ok(),
        Some(board.reviewable_now())
    );
    assert_eq!(usize::try_from(popover.total).ok(), Some(board.total()));
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
    // The status is the cause beneath the top-level error, so this fails if
    // the message stops at the first link of the chain.
    assert!(
        message.as_deref().is_some_and(|text| text.contains("500")),
        "a failure carries its cause: {message:?}"
    );
    // This crate never holds the token, so this guards `prtrack-github`'s
    // error text: nothing in the chain it hands back may print the token.
    assert!(
        message
            .as_deref()
            .is_some_and(|text| !text.contains(SECRET)),
        "a failure never carries the token: {message:?}"
    );
    Ok(())
}
