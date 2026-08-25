//! Sending the query, and following it across pages.

use std::time::Duration;

use prtrack_core::RepoRef;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, USER_AGENT};
use reqwest::{StatusCode, Url};
use serde::Serialize;

use crate::error::{GitHubError, Malformed, NestedList};
use crate::retry::RetryBudget;
use crate::token::Token;
use crate::wire;

/// GitHub's own GraphQL endpoint. A GitHub Enterprise host is set with
/// [`GitHubClient::with_endpoint`].
const GITHUB_GRAPHQL_ENDPOINT: &str = "https://api.github.com/graphql";

/// `mergeStateStatus` does not resolve without this media type.
const MERGE_INFO_PREVIEW: &str = "application/vnd.github.merge-info-preview+json";

/// GitHub rejects a request that does not identify itself.
const CLIENT_USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));

/// The query, shared with the fixture recording so the two cannot drift.
const QUERY: &str = include_str!("open_pull_requests.graphql");

/// How long one request may take before it is abandoned.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// How long to wait for the connection itself.
///
/// **Not covered by any test, deliberately.** The request timeout beside it is
/// pinned by a delayed response, but a *connect* timeout only fires when a TCP
/// handshake never completes, and the honest ways to provoke that — a
/// blackholed address, a firewalled port — depend on the network the suite
/// happens to run on. A test that hangs on someone else's machine would be
/// worse than this comment. Mutating it away leaves the suite green; that is
/// known, not overlooked.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a release download may take.
///
/// Longer than a query, because this is megabytes over whatever connection the
/// user has rather than a few kilobytes of JSON.
const ASSET_TIMEOUT: Duration = Duration::from_mins(5);

/// A published release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// The tag, as published — `v0.1.2`, leading `v` included.
    pub tag: String,
    /// The files attached to it.
    pub assets: Vec<Asset>,
}

/// One file attached to a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    /// Its filename, which is how a caller tells the platforms apart.
    pub name: String,
    /// The **API** url. Not the browser one: only this accepts the token, and
    /// a private repository refuses the other.
    pub url: String,
}

/// Pull requests per page. Fifty with full check context costs 3 GraphQL
/// points, which leaves the budget a non-issue at any repository size this
/// tool is pointed at.
const PAGE_SIZE: i64 = 50;

/// How many pages to follow before giving up on ever reaching the end.
const MAX_PAGES: usize = 100;

/// The header GitHub reports the remaining rate limit in.
const RATE_LIMIT_REMAINING: &str = "x-ratelimit-remaining";

/// A client for one GitHub API endpoint, holding one credential.
///
/// The credential lives in the underlying HTTP client's default headers, marked
/// sensitive, and is never held as text — see [`Token`]. `Debug` is written by
/// hand so that neither the headers nor anything derived from them can reach a
/// log.
pub struct GitHubClient {
    http: reqwest::Client,
    endpoint: Url,
    retry: RetryBudget,
    timeout: Duration,
}

impl std::fmt::Debug for GitHubClient {
    /// Prints the endpoint and the retry budget, and nothing that could carry
    /// the credential.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubClient")
            .field("endpoint", &self.endpoint.as_str())
            .field("retry", &self.retry)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl GitHubClient {
    /// Builds a client that authenticates with `token` against github.com.
    ///
    /// The preview media type `mergeStateStatus` needs, the user agent GitHub
    /// insists on, and the credential are set once as default headers, so no
    /// call site can forget one.
    ///
    /// # Errors
    ///
    /// Returns [`GitHubError::Transport`] if the underlying HTTP client cannot
    /// be built at all, which in practice means the TLS backend failed to
    /// initialise.
    pub fn new(token: &Token) -> Result<Self, GitHubError> {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, token.credential());
        headers.insert(ACCEPT, HeaderValue::from_static(MERGE_INFO_PREVIEW));
        headers.insert(USER_AGENT, HeaderValue::from_static(CLIENT_USER_AGENT));

        let http = reqwest::Client::builder()
            .default_headers(headers)
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .map_err(GitHubError::Transport)?;

        Ok(Self {
            http,
            endpoint: checked_endpoint(GITHUB_GRAPHQL_ENDPOINT)?,
            retry: RetryBudget::default(),
            timeout: DEFAULT_TIMEOUT,
        })
    }

    /// Builds a client with a token read from the environment.
    ///
    /// # Errors
    ///
    /// Returns whatever [`Token::from_env`] or [`GitHubClient::new`] reports.
    pub fn from_env() -> Result<Self, GitHubError> {
        Self::new(&Token::from_env()?)
    }

    /// Points the client at another endpoint, such as a GitHub Enterprise host.
    ///
    /// # Errors
    ///
    /// Returns [`GitHubError::UnusableEndpoint`] unless `endpoint` is an
    /// absolute `http` or `https` URL with no credentials in it. Credentials
    /// are refused rather than stripped because a URL travels into the text of
    /// a transport error and out to a log from there.
    pub fn with_endpoint(self, endpoint: &str) -> Result<Self, GitHubError> {
        Ok(Self {
            endpoint: checked_endpoint(endpoint)?,
            ..self
        })
    }

    /// Replaces the retry budget.
    #[must_use]
    pub fn with_retry_budget(self, retry: RetryBudget) -> Self {
        Self { retry, ..self }
    }

    /// Replaces the per-request timeout.
    ///
    /// The timeout covers a whole attempt, so a hung connection costs at most
    /// this long rather than stalling a poll loop until someone notices.
    #[must_use]
    pub fn with_timeout(self, timeout: Duration) -> Self {
        Self { timeout, ..self }
    }

    /// A published release: its tag, and the files attached to it.
    ///
    /// Same authentication as everything else here, which is the point — a
    /// private repository's assets are reachable by exactly the people who can
    /// install from them, with no second credential and nothing hosted.
    ///
    /// # Errors
    ///
    /// `404` — no release published — is [`Ok(None)`](Option::None) rather than
    /// a failure. Anything else is reported.
    pub async fn latest_release_detail(
        &self,
        repo: &RepoRef,
    ) -> Result<Option<Release>, GitHubError> {
        let Some(body) = self.release_json(repo).await? else {
            return Ok(None);
        };
        let Some(tag) = body
            .get("tag_name")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
        else {
            return Ok(None);
        };
        let assets = body
            .get("assets")
            .and_then(serde_json::Value::as_array)
            .map(|assets| {
                assets
                    .iter()
                    .filter_map(|asset| {
                        Some(Asset {
                            name: asset.get("name")?.as_str()?.to_owned(),
                            url: asset.get("url")?.as_str()?.to_owned(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(Some(Release { tag, assets }))
    }

    /// Downloads one release asset.
    ///
    /// The URL must be the API url from [`Asset::url`], not the browser one:
    /// only the API url accepts the token, and a private repository refuses the
    /// other.
    ///
    /// # Errors
    ///
    /// [`GitHubError::UnexpectedStatus`] for anything GitHub refuses, and
    /// [`GitHubError::Transport`] if the body cannot be read.
    pub async fn download_asset(&self, url: &str) -> Result<Vec<u8>, GitHubError> {
        let response = self
            .http
            .get(url)
            // Without this GitHub answers with the asset's *metadata* as JSON,
            // which downloads successfully and is not the file.
            .header(ACCEPT, "application/octet-stream")
            .timeout(ASSET_TIMEOUT)
            .send()
            .await
            .map_err(GitHubError::Transport)?;

        if !response.status().is_success() {
            return Err(GitHubError::UnexpectedStatus {
                status: response.status().as_u16(),
            });
        }
        Ok(response
            .bytes()
            .await
            .map_err(GitHubError::Transport)?
            .to_vec())
    }

    /// The `releases/latest` document, or `None` where nothing is published.
    async fn release_json(&self, repo: &RepoRef) -> Result<Option<serde_json::Value>, GitHubError> {
        let base = self
            .endpoint
            .as_str()
            .strip_suffix("/graphql")
            .unwrap_or(self.endpoint.as_str());
        let url = format!(
            "{base}/repos/{}/{}/releases/latest",
            repo.owner(),
            repo.name()
        );

        let response = self
            .http
            .get(&url)
            .header(ACCEPT, "application/vnd.github+json")
            .timeout(self.timeout)
            .send()
            .await
            .map_err(GitHubError::Transport)?;

        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(GitHubError::UnexpectedStatus {
                status: response.status().as_u16(),
            });
        }
        let raw = response.bytes().await.map_err(GitHubError::Transport)?;
        serde_json::from_slice(&raw)
            .map(Some)
            .map_err(GitHubError::Decode)
    }

    /// The tag of the newest published release, or `None` where there is none.
    ///
    /// Reuses this client's token, which is the point: `prtrack` already
    /// requires a token that can read the repository, so a private repository's
    /// releases are visible to exactly the people who can install from them. No
    /// public endpoint, no second credential, and nothing to run.
    ///
    /// # Errors
    ///
    /// A repository with no releases yet answers `404`, which is `Ok(None)`
    /// rather than a failure — "nothing published" is an answer. Anything else
    /// is reported, so a caller can tell a refused token from an empty release
    /// list. Callers that treat this as a courtesy should discard the error;
    /// a version check must never be why the tool did not run.
    ///
    /// The REST base is derived from the configured GraphQL endpoint by
    /// dropping `/graphql`, which is right for github.com. GitHub Enterprise
    /// serves REST under `/api/v3` and is **untested here**.
    pub async fn latest_release(&self, repo: &RepoRef) -> Result<Option<String>, GitHubError> {
        let base = self
            .endpoint
            .as_str()
            .strip_suffix("/graphql")
            .unwrap_or(self.endpoint.as_str());
        let url = format!(
            "{base}/repos/{}/{}/releases/latest",
            repo.owner(),
            repo.name()
        );

        let response = self
            .http
            .get(&url)
            .header(ACCEPT, "application/vnd.github+json")
            .timeout(self.timeout)
            .send()
            .await
            .map_err(GitHubError::Transport)?;

        // Not retried, unlike the board fetch. This is a courtesy check on a
        // one-hour cache; a failed attempt costs nothing and the next one is
        // along shortly, where retrying would spend the rate limit that the
        // actual work needs.
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(GitHubError::UnexpectedStatus {
                status: response.status().as_u16(),
            });
        }

        // Read, then parse, so a transport failure and a malformed body stay
        // in different buckets — `reqwest`'s combined `json()` collapses them.
        let raw = response.bytes().await.map_err(GitHubError::Transport)?;
        let body: serde_json::Value = serde_json::from_slice(&raw).map_err(GitHubError::Decode)?;
        Ok(body
            .get("tag_name")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned))
    }

    /// Fetches every open pull request in `repo`, following the list across as
    /// many pages as it takes.
    ///
    /// An `Ok` with nothing in it means the repository has no open pull
    /// requests. Every way of failing to find that out is an `Err`, so an empty
    /// board can never mean "GitHub is down".
    ///
    /// # Errors
    ///
    /// Returns [`GitHubError::RepositoryUnavailable`] when the repository is
    /// not visible to the token, [`GitHubError::Unauthorized`] or
    /// [`GitHubError::RateLimited`] when GitHub refuses,
    /// [`GitHubError::GaveUp`] when a failure that looked transient did not
    /// clear within the retry budget, and [`GitHubError::TooManyPages`] when
    /// GitHub keeps promising another page.
    pub async fn fetch_open_pull_requests(
        &self,
        repo: &RepoRef,
    ) -> Result<OpenPullRequests, GitHubError> {
        // Only a list that moved is read again, and it is read from the start:
        // the pages that did arrive cannot be patched up, because the one that
        // was missed is not at the end. Everything else propagates on its first
        // answer, so a dead API still costs one budget rather than one squared.
        let mut attempts = 0_u32;
        let mut moved = None;

        for backoff in self.retry.schedule() {
            tokio::time::sleep(backoff).await;
            attempts += 1;
            match self.read_whole_list(repo).await {
                Ok(open) => return Ok(open),
                Err(GitHubError::Malformed(short @ Malformed::ListMoved { .. })) => {
                    moved = Some(short);
                }
                Err(other) => return Err(other),
            }
        }

        Err(GitHubError::GaveUp {
            attempts,
            source: Box::new(GitHubError::Malformed(moved.unwrap_or(
                Malformed::ListMoved {
                    collected: 0,
                    first_total: 0,
                    last_total: 0,
                },
            ))),
        })
    }

    /// One pass over every page, refusing a set that does not add up.
    async fn read_whole_list(&self, repo: &RepoRef) -> Result<OpenPullRequests, GitHubError> {
        let first = self.fetch_page(repo, None).await?;
        let first_page = first.query;
        let first_total = reported_total_of(&first_page);
        let mut last_total = first_total;
        let mut pull_requests = first.pull_requests;
        let mut next = first.next;
        let mut pages = 1_usize;

        while let NextPage::Follow(cursor) = next {
            if pages >= MAX_PAGES {
                return Err(GitHubError::TooManyPages { pages });
            }
            let page = self.fetch_page(repo, Some(&cursor)).await?;
            last_total = reported_total_of(&page.query);
            pull_requests.extend(page.pull_requests);
            next = page.next;
            pages += 1;
        }

        // The three-way check the fixture recorder already makes, and for the
        // same reason: `hasNextPage: false` says the cursor reached the end,
        // not that everything was seen.
        let collected = pull_requests.len();
        let agreed = first_total
            .zip(last_total)
            .is_some_and(|(first, last)| first == last && i64::try_from(collected) == Ok(first));
        if !agreed {
            return Err(GitHubError::Malformed(Malformed::ListMoved {
                collected,
                first_total: first_total.unwrap_or(-1),
                last_total: last_total.unwrap_or(-1),
            }));
        }

        Ok(OpenPullRequests {
            repo: repo.clone(),
            first_page,
            pull_requests,
        })
    }

    /// Fetches one page, retrying whatever looked transient.
    ///
    /// The first attempt is made outside the loop so that "the budget was
    /// spent" is a state the types can express without a placeholder error
    /// standing in for a failure that never happened.
    async fn fetch_page(&self, repo: &RepoRef, after: Option<&str>) -> Result<Page, GitHubError> {
        let mut attempts = 1_u32;
        let mut failure = match self.attempt(repo, after).await {
            Ok(page) => return Ok(page),
            Err(AttemptFailure::Permanent(error)) => return Err(error),
            Err(AttemptFailure::Transient(error)) => error,
        };

        for backoff in self.retry.schedule().skip(1) {
            tokio::time::sleep(backoff).await;
            attempts += 1;
            failure = match self.attempt(repo, after).await {
                Ok(page) => return Ok(page),
                Err(AttemptFailure::Permanent(error)) => return Err(error),
                Err(AttemptFailure::Transient(error)) => error,
            };
        }

        Err(GitHubError::GaveUp {
            attempts,
            source: Box::new(failure),
        })
    }

    /// One request, and one reading of what came back.
    async fn attempt(&self, repo: &RepoRef, after: Option<&str>) -> Result<Page, AttemptFailure> {
        let response = self
            .http
            .post(self.endpoint.clone())
            .timeout(self.timeout)
            .json(&Request::page(repo, after))
            .send()
            .await
            .map_err(unreachable_host)?;

        let status = response.status();
        if !status.is_success() {
            return Err(refusal(status, response.headers()));
        }

        let body = response.bytes().await.map_err(unreachable_host)?;
        let envelope: wire::Envelope = serde_json::from_slice(&body)
            .map_err(|source| AttemptFailure::Transient(GitHubError::Decode(source)))?;

        Page::read(data_of(envelope)?, repo)
    }
}

/// Whether a failed attempt is worth repeating.
///
/// An enum rather than a `bool` beside the error, so that classifying a failure
/// and reporting it cannot drift apart: every failure is built as one or the
/// other at the point where the reason is known.
enum AttemptFailure {
    /// Worth another try — a hung connection, a 5xx, a page with a hole in it.
    Transient(GitHubError),
    /// No amount of retrying will change the answer.
    Permanent(GitHubError),
}

/// Whether the list continues, and where from.
///
/// GitHub can say "another page follows" and name no cursor to fetch it with.
/// That combination has no place in this enum, so [`Page::read`] has to decide
/// what it means once — it refuses the page — instead of every caller quietly
/// treating it as the end of the list.
enum NextPage {
    /// Another page follows, at this cursor.
    Follow(String),
    /// That was the last page.
    End,
}

/// One page whose every part the caller depends on is known to be there.
///
/// The only way to build one is [`Page::read`], so a page cannot be assembled
/// out of a response that was missing its `pageInfo`, missing its `nodes`, or
/// carrying a null where a pull request should have been.
struct Page {
    /// The response as it arrived, minus the nodes, which moved into
    /// `pull_requests` rather than being held twice.
    query: wire::Query,
    /// The pull requests on this page, with no holes in the list.
    pull_requests: Vec<wire::PullRequest>,
    /// Whether to ask for more.
    next: NextPage,
}

impl Page {
    /// Reads a page, refusing anything that would silently shorten the list.
    fn read(mut query: wire::Query, repo: &RepoRef) -> Result<Self, AttemptFailure> {
        let Some(repository) = query.repository.as_mut() else {
            return Err(AttemptFailure::Permanent(
                GitHubError::RepositoryUnavailable { repo: repo.clone() },
            ));
        };
        let connection = &mut repository.pull_requests;

        let next = match connection.page_info.as_ref() {
            None => return Err(incomplete(Malformed::NoPageInfo)),
            Some(page_info) if !page_info.has_next_page => NextPage::End,
            Some(page_info) => match page_info.end_cursor.as_deref() {
                Some(cursor) => NextPage::Follow(cursor.to_owned()),
                None => return Err(incomplete(Malformed::NoCursor)),
            },
        };

        let Some(nodes) = connection.nodes.take() else {
            return Err(incomplete(Malformed::NoNodes));
        };
        let pull_requests = nodes
            .into_iter()
            .map(|node| node.ok_or(Malformed::NullNode))
            .collect::<Result<Vec<_>, Malformed>>()
            .map_err(incomplete)?;

        if let Some(short) = truncation(&query, &pull_requests) {
            return Err(incomplete(short));
        }
        if let Some(unreadable) = misparsed_member(&pull_requests) {
            return Err(incomplete(unreadable));
        }

        Ok(Self {
            query,
            pull_requests,
            next,
        })
    }
}

/// Whether any connection this crate reads whole came back short.
///
/// `Page::read` guards the outer list; these are the nested ones, every one of
/// them capped by the query. The first shortfall found is the one reported —
/// a page missing one entry is refused whether or not it is missing others.
fn truncation(query: &wire::Query, pull_requests: &[wire::PullRequest]) -> Option<Malformed> {
    let rules = query.repository.as_ref().and_then(|repository| {
        shortfall(
            &repository.branch_protection_rules,
            NestedList::ProtectionRules,
        )
    });
    if rules.is_some() {
        return rules;
    }

    pull_requests.iter().find_map(shortfall_in)
}

/// The first connection on one pull request that came back short.
fn shortfall_in(pull_request: &wire::PullRequest) -> Option<Malformed> {
    let listed = [
        shortfall(&pull_request.review_requests, NestedList::ReviewRequests),
        shortfall(&pull_request.latest_reviews, NestedList::LatestReviews),
        shortfall(&pull_request.review_threads, NestedList::ReviewThreads),
    ];
    if let Some(short) = listed.into_iter().flatten().next() {
        return Some(short);
    }

    pull_request
        .commits
        .nodes
        .iter()
        .flatten()
        .flatten()
        .filter_map(|node| node.commit.status_check_rollup.as_ref())
        .find_map(|rollup| shortfall(&rollup.contexts, NestedList::CheckContexts))
}

/// The first union member that fell through to its fallback arm despite being
/// one this crate names.
///
/// See [`wire::CheckContext::misparsed`] for why an untagged fallback makes
/// this necessary: it is the arm that lets an unfamiliar member survive, and
/// the same property means a familiar one whose fields changed lands there too,
/// silently.
fn misparsed_member(pull_requests: &[wire::PullRequest]) -> Option<Malformed> {
    let reviewers = pull_requests
        .iter()
        .flat_map(|pull_request| pull_request.review_requests.nodes.iter())
        .flatten()
        .flatten()
        .filter_map(|request| request.requested_reviewer.as_ref())
        .find_map(wire::RequestedReviewer::misparsed);

    let contexts = || {
        pull_requests
            .iter()
            .flat_map(|pull_request| pull_request.commits.nodes.iter())
            .flatten()
            .flatten()
            .filter_map(|node| node.commit.status_check_rollup.as_ref())
            .flat_map(|rollup| rollup.contexts.nodes.iter())
            .flatten()
            .flatten()
            .find_map(wire::CheckContext::misparsed)
    };

    reviewers
        .or_else(contexts)
        .map(|typename| Malformed::UnreadableMember {
            typename: typename.to_owned(),
        })
}

/// Whether one connection returned every entry it says exists.
///
/// A connection with no total is refused rather than assumed whole. The query
/// asks for a total on every connection this runs against, so an absent one is
/// not "we did not ask" — it is an answer that cannot be checked, and an
/// unverifiable list is exactly the shape of a silently short one.
fn shortfall<T>(connection: &wire::Connection<T>, what: NestedList) -> Option<Malformed> {
    let counted = connection.nodes.as_ref().map_or(0, Vec::len);
    match connection.total_count {
        None => Some(Malformed::Uncountable { what }),
        Some(total) if i64::try_from(counted) == Ok(total) => None,
        Some(total) => Some(Malformed::Truncated {
            what,
            counted,
            total,
        }),
    }
}

/// A page that arrived with a hole in it. Worth another try: measured against
/// the live API, roughly one page in nine came back unusable and the next
/// attempt was fine.
const fn incomplete(malformed: Malformed) -> AttemptFailure {
    AttemptFailure::Transient(GitHubError::Malformed(malformed))
}

/// A request that never got an answer. Always worth another try — a timeout, a
/// refused connection, and a dropped socket are all momentary.
const fn unreachable_host(source: reqwest::Error) -> AttemptFailure {
    AttemptFailure::Transient(GitHubError::Transport(source))
}

/// What GitHub refusing with `status` means, and whether asking again could
/// help.
fn refusal(status: StatusCode, headers: &HeaderMap) -> AttemptFailure {
    if status == StatusCode::UNAUTHORIZED {
        AttemptFailure::Permanent(GitHubError::Unauthorized)
    } else if status == StatusCode::FORBIDDEN {
        // GitHub reports a spent rate limit as a 403 with the remaining count
        // at zero, and everything else it forbids as a bare 403.
        if rate_limit_spent(headers) {
            AttemptFailure::Permanent(GitHubError::RateLimited)
        } else {
            AttemptFailure::Permanent(GitHubError::Forbidden)
        }
    } else if status == StatusCode::TOO_MANY_REQUESTS {
        // Deliberately not retried. A secondary rate limit is measured in
        // minutes, and retrying inside a second is what earns one in the first
        // place.
        AttemptFailure::Permanent(GitHubError::RateLimited)
    } else if status.is_server_error() {
        AttemptFailure::Transient(GitHubError::UnexpectedStatus {
            status: status.as_u16(),
        })
    } else {
        AttemptFailure::Permanent(GitHubError::UnexpectedStatus {
            status: status.as_u16(),
        })
    }
}

/// Whether GitHub's headers say the rate limit is spent.
fn rate_limit_spent(headers: &HeaderMap) -> bool {
    headers
        .get(RATE_LIMIT_REMAINING)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|remaining| remaining.trim() == "0")
}

/// The data half of an envelope, or the reason there is none.
///
/// Errors alongside data are ignored: GraphQL returns partial data with an
/// explanation of what it could not resolve, and the parts this crate depends
/// on are checked by [`Page::read`] regardless of what GitHub said about the
/// parts it does not.
///
/// Errors *instead of* data are transient. The commonest one in practice is
/// GitHub answering 200 with "something went wrong while executing your query",
/// which is a timeout on their side and clears on the next attempt. A query
/// that is genuinely wrong costs the budget and then reports itself, which is
/// a fair price for not giving up on a flake.
fn data_of(envelope: wire::Envelope) -> Result<wire::Query, AttemptFailure> {
    if let Some(query) = envelope.data {
        // Data came back, but an error may have nulled part of it on the way.
        return nulled_field(envelope.errors.as_deref()).map_or(Ok(query), |path| {
            Err(AttemptFailure::Transient(GitHubError::Malformed(
                Malformed::ErrorNulledField { path },
            )))
        });
    }

    let errors = envelope.errors.unwrap_or_default();

    // A spent primary rate limit arrives here rather than as a 403 or a 429:
    // GitHub answers 200 with an error-only body, so the status code says
    // nothing. It is the one failure in this shape that retrying cannot help —
    // the window resets on the hour — and it is worth naming rather than
    // reporting as a budget spent, because "wait" and "your query is broken"
    // are different instructions to whoever is reading.
    if errors
        .iter()
        .any(|error| matches!(error.error_type, Some(wire::GraphQlErrorType::RateLimited)))
    {
        return Err(AttemptFailure::Permanent(GitHubError::RateLimited));
    }

    let messages: Vec<String> = errors.into_iter().map(|error| error.message).collect();

    Err(AttemptFailure::Transient(if messages.is_empty() {
        GitHubError::Malformed(Malformed::NoData)
    } else {
        GitHubError::GraphQl { messages }
    }))
}

/// The path of the first error that nulled a field this crate reads.
///
/// An error carrying a path nulled the field at that path — that is what a
/// GraphQL path means — and the response came back around the hole. The hole is
/// the problem: a nulled `statusCheckRollup` is indistinguishable from a pull
/// request whose CI never ran, and reads as a green board.
///
/// A path of exactly `repository` is left alone. That is the shape a
/// nonexistent or invisible repository arrives in, and it is already reported
/// as [`GitHubError::RepositoryUnavailable`], which says something truer and
/// does not retry. Everything else is worth another attempt, since a nulled
/// field is usually GitHub timing out on one resolver.
///
/// Errors with no path nulled nothing in particular; they are left to the
/// no-data branch below.
fn nulled_field(errors: Option<&[wire::GraphQlError]>) -> Option<String> {
    errors?
        .iter()
        .filter_map(|error| error.path.as_ref())
        .filter(|path| !is_the_repository_itself(path))
        .map(|path| render_path(path))
        .next()
}

/// Whether a path points at the repository as a whole rather than into it.
fn is_the_repository_itself(path: &[wire::PathSegment]) -> bool {
    matches!(path, [wire::PathSegment::Field(field)] if field == "repository")
}

/// A GraphQL path as a reader would write it.
fn render_path(path: &[wire::PathSegment]) -> String {
    path.iter()
        .map(|segment| match segment {
            wire::PathSegment::Field(name) => name.clone(),
            wire::PathSegment::Index(index) => index.to_string(),
        })
        .collect::<Vec<String>>()
        .join(".")
}

/// The total a page reported, if it reported one.
fn reported_total_of(page: &wire::Query) -> Option<i64> {
    page.repository
        .as_ref()
        .and_then(|repository| repository.pull_requests.total_count)
}

/// Parses an endpoint and refuses one that could carry a credential into a log.
fn checked_endpoint(raw: &str) -> Result<Url, GitHubError> {
    let url = Url::parse(raw).map_err(|_source| GitHubError::UnusableEndpoint)?;
    let usable = matches!(url.scheme(), "http" | "https")
        && url.username().is_empty()
        && url.password().is_none();
    usable.then_some(url).ok_or(GitHubError::UnusableEndpoint)
}

/// The variables the query is sent with.
#[derive(Debug, Serialize)]
struct Variables<'a> {
    owner: &'a str,
    name: &'a str,
    first: i64,
    after: Option<&'a str>,
}

/// One GraphQL request body.
#[derive(Debug, Serialize)]
struct Request<'a> {
    query: &'a str,
    variables: Variables<'a>,
}

impl<'a> Request<'a> {
    /// The request for one page of `repo`'s open pull requests.
    fn page(repo: &'a RepoRef, after: Option<&'a str>) -> Self {
        Self {
            query: QUERY,
            variables: Variables {
                owner: repo.owner(),
                name: repo.name(),
                first: PAGE_SIZE,
                after,
            },
        }
    }
}

/// Every open pull request in one repository, still in wire shape.
///
/// This is where this crate stops today. The conversion into `prtrack-core`'s
/// domain types is a separate task, so nothing here interprets what it holds:
/// the accessors below report what arrived and how much of it, and the wire
/// types themselves are private to this crate so no caller can build logic on
/// their shape before that conversion exists.
#[derive(Debug)]
pub struct OpenPullRequests {
    /// The repository this was fetched for.
    ///
    /// Carried rather than read back out of the response: the query selects no
    /// `nameWithOwner`, and the caller's own reference is the one their board
    /// is scoped by. It scopes every pull request number into something that
    /// identifies a pull request on its own.
    pub(crate) repo: RepoRef,
    /// The first page as it arrived, minus its pull requests, which moved into
    /// the flat list below rather than being held twice. The viewer, the rate
    /// limit, and the branch protection rules the whole board is read against
    /// all come from here.
    pub(crate) first_page: wire::Query,
    /// Every open pull request, in the order the pages returned them.
    ///
    /// The last page's own `totalCount` is deliberately *not* kept beside the
    /// first page's. Both were read, and both had to agree with this list's
    /// length before the value could be built at all — so a second copy would
    /// only be a number that cannot differ.
    pub(crate) pull_requests: Vec<wire::PullRequest>,
}

impl OpenPullRequests {
    /// How many pull requests came back across every page.
    ///
    /// Zero is a real answer meaning "nothing is open", and it is reached only
    /// through `Ok`. A repository that could not be read is an `Err`, never an
    /// empty list.
    #[must_use]
    pub const fn collected(&self) -> usize {
        self.pull_requests.len()
    }

    /// How many open pull requests GitHub said there were when the first page
    /// was fetched.
    ///
    /// Signed, because the wire format is, and this crate does not narrow what
    /// it did not check.
    ///
    /// It cannot differ from [`OpenPullRequests::collected`]: a fetch whose
    /// pages did not add up to this number is refused rather than returned,
    /// because a pull request stepped over by the cursor and a repository with
    /// one fewer pull request produce the same short list, and only the
    /// arithmetic tells them apart. Both readings of the total, from the first
    /// page and the last, had to agree as well.
    #[must_use]
    pub fn reported_total(&self) -> Option<i64> {
        self.first_page
            .repository
            .as_ref()
            .and_then(|repository| repository.pull_requests.total_count)
    }

    /// The repository these pull requests are open against.
    #[must_use]
    pub const fn repo(&self) -> &RepoRef {
        &self.repo
    }

    /// The login the token authenticates as.
    #[must_use]
    pub fn viewer_login(&self) -> &str {
        &self.first_page.viewer.login
    }

    /// GraphQL rate limit points left when the first page was fetched, if
    /// GitHub reported them.
    #[must_use]
    pub fn rate_limit_remaining(&self) -> Option<i64> {
        self.first_page
            .rate_limit
            .as_ref()
            .map(|rate_limit| rate_limit.remaining)
    }
}

#[cfg(test)]
mod tests {
    use super::{GitHubClient, MAX_PAGES, MERGE_INFO_PREVIEW, QUERY};
    use crate::error::{GitHubError, Malformed, NestedList};
    use crate::retry::RetryBudget;
    use crate::token::Token;
    use crate::wire::{PAGE_1, PAGE_1_END_CURSOR, PAGE_2, TOTAL_OPEN};
    use prtrack_core::RepoRef;
    use std::collections::BTreeSet;
    use std::num::NonZeroU32;
    use std::sync::Mutex;
    use std::time::Duration;
    use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

    /// A token-shaped string no real account issued, checked for by substring
    /// in the redaction tests.
    const SECRET: &str = "ghp_0123456789abcdefSECRETzzzz";

    /// The repository the recordings claim to come from, after redaction.
    const REPO: &str = "example-org/example-repo";

    /// Three attempts, the same as the default, so a test that spends the
    /// budget spends a known number of them.
    const ATTEMPTS: u32 = 3;

    /// Answers a scripted sequence of responses, one per request.
    ///
    /// A single mock with a script rather than several mocks with matchers, so
    /// that the order responses arrive in is decided here and not by whatever
    /// order the mock server happens to try its matchers in. The last response
    /// repeats once the script runs out, which is what lets a test drive the
    /// page cap without writing a hundred entries.
    struct Script {
        remaining: Mutex<Vec<ResponseTemplate>>,
    }

    impl Script {
        /// Answers `responses` in order.
        fn of(responses: Vec<ResponseTemplate>) -> Self {
            let mut reversed = responses;
            reversed.reverse();
            Self {
                remaining: Mutex::new(reversed),
            }
        }
    }

    impl Respond for Script {
        fn respond(&self, _request: &Request) -> ResponseTemplate {
            let next = self.remaining.lock().ok().and_then(|mut remaining| {
                match remaining.len() {
                    0 => None,
                    // The last one stays put and answers everything after it.
                    1 => remaining.last().cloned(),
                    _ => remaining.pop(),
                }
            });
            next.unwrap_or_else(|| ResponseTemplate::new(500))
        }
    }

    /// A 200 carrying `body` as JSON, the way GitHub's GraphQL endpoint answers.
    fn ok(body: &str) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_raw(body.to_owned(), "application/json")
    }

    /// What a fetch did, in the terms the assertions below are written in.
    ///
    /// A total type on purpose. Setting the harness up involves four fallible
    /// constructors that cannot actually fail on these constants, and rather
    /// than unwrap them — or panic, which would be the same thing wearing a
    /// message — a setup that did not run becomes
    /// [`Outcome::NotAttempted`], which satisfies no assertion in this module.
    #[derive(Debug)]
    enum Outcome {
        /// The fetch succeeded, reduced to what the tests read.
        Fetched {
            /// Pull requests collected across every page.
            collected: usize,
            /// The total GitHub itself claimed on the first page.
            reported_total: Option<i64>,
            /// The login the token authenticated as.
            viewer: String,
            /// Rate limit points left, if GitHub reported them.
            rate_limit_remaining: Option<i64>,
        },
        /// The fetch ran and failed.
        Failed(GitHubError),
        /// The harness never got as far as fetching.
        NotAttempted,
    }

    /// A client pointed at `server` with no backoff, so a test that exercises
    /// the retry path does not spend real seconds asleep.
    fn client_for(server: &MockServer) -> Result<GitHubClient, GitHubError> {
        let budget = RetryBudget::new(
            NonZeroU32::new(ATTEMPTS).unwrap_or(NonZeroU32::MIN),
            Duration::ZERO,
        );
        Ok(GitHubClient::new(&Token::parse(SECRET)?)?
            .with_endpoint(&format!("{}/graphql", server.uri()))?
            .with_retry_budget(budget))
    }

    /// Runs one fetch against a scripted server, returning what it did and the
    /// requests the client actually sent.
    async fn fetch(responses: Vec<ResponseTemplate>) -> (Outcome, Vec<Request>) {
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::method("POST"))
            .respond_with(Script::of(responses))
            .mount(&server)
            .await;

        let outcome = run(client_for(&server)).await;
        let sent = server.received_requests().await.unwrap_or_default();
        (outcome, sent)
    }

    /// Fetches with `client`, reducing whatever happens to an [`Outcome`].
    async fn run(client: Result<GitHubClient, GitHubError>) -> Outcome {
        let (Ok(client), Ok(repo)) = (client, RepoRef::parse(REPO)) else {
            return Outcome::NotAttempted;
        };
        match client.fetch_open_pull_requests(&repo).await {
            Ok(open) => Outcome::Fetched {
                collected: open.collected(),
                reported_total: open.reported_total(),
                viewer: open.viewer_login().to_owned(),
                rate_limit_remaining: open.rate_limit_remaining(),
            },
            Err(error) => Outcome::Failed(error),
        }
    }

    /// The `variables` object of a request the client sent.
    fn variables(request: &Request) -> serde_json::Value {
        serde_json::from_slice::<serde_json::Value>(&request.body)
            .ok()
            .and_then(|body| body.get("variables").cloned())
            .unwrap_or(serde_json::Value::Null)
    }

    /// One variable of a request the client sent.
    fn variable(sent: &[Request], index: usize, name: &str) -> serde_json::Value {
        sent.get(index)
            .map(variables)
            .and_then(|variables| variables.get(name).cloned())
            .unwrap_or(serde_json::Value::Null)
    }

    /// A recorded page with its pull request connection replaced, for the cases
    /// the live API did not produce.
    ///
    /// Everything else in the page is still the recording, so a test written
    /// against this is exercising the real shape with one part varied.
    fn page_1_with(pull_requests: &serde_json::Value) -> String {
        let mut body =
            serde_json::from_str::<serde_json::Value>(PAGE_1).unwrap_or(serde_json::Value::Null);
        if let Some(slot) = body.pointer_mut("/data/repository/pullRequests") {
            *slot = pull_requests.clone();
        }
        body.to_string()
    }

    #[tokio::test]
    async fn a_status_no_retry_can_change_is_reported_on_the_first_attempt() {
        // 404 and 422 never turn into a 200. Spending the budget on them costs
        // three times the latency to reach the same answer, and against a real
        // secondary-rate-limit window it is how a client earns a longer one.
        for status in [400_u16, 404, 422] {
            let (outcome, sent) = fetch(vec![ResponseTemplate::new(status)]).await;

            assert!(
                matches!(
                    &outcome,
                    Outcome::Failed(GitHubError::UnexpectedStatus { status: reported })
                        if *reported == status
                ),
                "expected {status} to be reported as itself, got {outcome:?}"
            );
            assert_eq!(sent.len(), 1, "{status} must not be retried");
        }
    }

    #[tokio::test]
    async fn a_page_that_fails_after_the_first_is_never_returned_as_a_short_list() {
        // The failure mode this crate exists to avoid: 19 of 38 pull requests,
        // returned as `Ok`, is indistinguishable from a repository that merges
        // twice as fast as it does. Whatever was already collected is discarded
        // when a later page cannot be had.
        let cases = [
            ("a permanent refusal", ResponseTemplate::new(401), 2),
            (
                "a transient failure that never clears",
                ResponseTemplate::new(502),
                1 + ATTEMPTS as usize,
            ),
        ];

        for (what, failure, expected_requests) in cases {
            let (outcome, sent) = fetch(vec![ok(PAGE_1), failure]).await;

            assert!(
                matches!(&outcome, Outcome::Failed(_)),
                "{what} on page two must fail the whole fetch, got {outcome:?}"
            );
            assert_eq!(
                sent.len(),
                expected_requests,
                "{what}: the good first page plus the attempts at the second"
            );
        }
    }

    // # What the four query checks below establish, and what they do not
    //
    // They exist because the document was verified by nothing: the fixtures are
    // recordings, the mock server ignores the request body, and `QUERY` was
    // compared against nothing at all. Eight separate mutations to it — four
    // deleted fields, three changed arguments, and one alias — left the entire
    // suite passing.
    //
    // **What they establish.** That the query and the recordings agree: every
    // field a recording contains is still asked for, every field asked for is
    // one a recording showed, no selection is aliased, and the four arguments
    // that decide *which* pull requests arrive are the ones intended.
    //
    // **What they do not, and this is the important half.**
    //
    // 1. The recordings were produced *by this query*, through `record.sh`. So
    //    the two agree by construction about anything a recording is silent on.
    //    This is a drift detector — an edit to one without the other fails —
    //    and not a proof that the query is right.
    // 2. A field the query asks for that GitHub never returns for the recorded
    //    repository is invisible: it can only be allowlisted, as `slug` is,
    //    never verified.
    // 3. A field GitHub has *added* is invisible until someone re-records.
    // 4. The key-set checks cannot see arguments at all. The explicit list is
    //    exactly as good as the four entries in it, and a fifth argument added
    //    to the query is checked by nobody.
    // 5. None of it reaches the network, so none of it can prove the query is
    //    *right* — only that it has not drifted from the recordings.

    /// The query document, with comments and argument lists removed and every
    /// brace flattened to whitespace, so that what is left is field selections.
    ///
    /// Arguments are dropped rather than parsed because every legitimate colon
    /// in a GraphQL document lives inside them — variable definitions and
    /// argument names alike. What remains is checked for colons separately, and
    /// finding one means an alias.
    fn selections_only() -> String {
        let uncommented: String = QUERY
            .lines()
            .map(|line| line.split('#').next().unwrap_or_default())
            .collect::<Vec<&str>>()
            .join("\n");

        let mut depth = 0_usize;
        let mut flattened = String::with_capacity(uncommented.len());
        for character in uncommented.chars() {
            match character {
                '(' => depth += 1,
                ')' => depth = depth.saturating_sub(1),
                _ if depth > 0 => {}
                '{' | '}' | '.' => flattened.push(' '),
                _ => flattened.push(character),
            }
        }
        flattened
    }

    /// Every field the query selects, by the name it comes back under.
    fn query_selections() -> BTreeSet<String> {
        let flattened = selections_only();
        let mut tokens = flattened.split_whitespace();
        let mut selections = BTreeSet::new();
        while let Some(token) = tokens.next() {
            match token {
                // The operation keyword and its name, and a fragment's type
                // condition, are not fields.
                "query" | "on" => {
                    tokens.next();
                }
                field => {
                    selections.insert(field.to_owned());
                }
            }
        }
        selections
    }

    /// Every key a recorded response contains, at any depth.
    fn recorded_field_names() -> BTreeSet<String> {
        fn walk(value: &serde_json::Value, into: &mut BTreeSet<String>) {
            match value {
                serde_json::Value::Object(fields) => {
                    for (key, nested) in fields {
                        into.insert(key.clone());
                        walk(nested, into);
                    }
                }
                serde_json::Value::Array(entries) => {
                    for entry in entries {
                        walk(entry, into);
                    }
                }
                // A leaf carries no field names.
                serde_json::Value::Null
                | serde_json::Value::Bool(_)
                | serde_json::Value::Number(_)
                | serde_json::Value::String(_) => {}
            }
        }

        let mut names = BTreeSet::new();
        for raw in [PAGE_1, PAGE_2] {
            if let Ok(page) = serde_json::from_str::<serde_json::Value>(raw) {
                walk(&page, &mut names);
            }
        }
        // The envelope, not selections.
        names.remove("data");
        names.remove("errors");
        names
    }

    /// A field the query asks for that no recording contains.
    ///
    /// `slug` is selected under `... on Team`, and every review request in the
    /// recorded repository is addressed to a user, so the fragment never
    /// matched and the field never appeared. It is not dead: a CODEOWNERS team
    /// can be asked for a review at any time, and dropping the selection would
    /// lose that request entirely.
    const REQUESTED_BUT_NEVER_RETURNED: [&str; 1] = ["slug"];

    #[test]
    fn the_query_asks_for_every_field_the_recording_contains() {
        // Without this the document is verified by nothing: the fixtures are
        // recordings, the mock server ignores the request body, and `QUERY` is
        // otherwise compared against nothing at all. Deleting `startedAt`,
        // `mergeStateStatus`, `pageInfo` or `requiredStatusCheckContexts` from
        // it leaves every other test in this crate passing.
        let selections = query_selections();
        let missing: Vec<String> = recorded_field_names()
            .into_iter()
            .filter(|name| !selections.contains(name))
            .collect();
        assert!(
            missing.is_empty(),
            "the recording contains fields the query no longer asks for: {missing:?}"
        );
    }

    #[test]
    fn the_query_asks_for_nothing_the_recording_does_not_contain() {
        let recorded = recorded_field_names();
        let unexplained: Vec<String> = query_selections()
            .into_iter()
            .filter(|name| !recorded.contains(name))
            .filter(|name| !REQUESTED_BUT_NEVER_RETURNED.contains(&name.as_str()))
            .collect();
        assert!(
            unexplained.is_empty(),
            "the query selects fields no recording shows, so nothing here \
             establishes they are real: {unexplained:?}"
        );
    }

    #[test]
    fn the_query_uses_no_aliases() {
        // Every legitimate colon lives inside an argument list, which
        // `selections_only` has already removed. One left over is an alias, and
        // an alias renames the response key — `x: statusCheckRollup` makes the
        // field arrive under `x`, silently emptying every check on every pull
        // request while the field-coverage check above still sees the name.
        let flattened = selections_only();
        assert!(
            !flattened.contains(':'),
            "an alias changes the key a field comes back under: {flattened}"
        );
    }

    #[test]
    fn the_query_asks_for_the_arguments_that_decide_what_comes_back() {
        // A key-set check cannot see arguments at all. These four decide which
        // pull requests arrive, in what order, and how paging works, and every
        // one of them can be changed without any other test noticing.
        let normalised = QUERY.split_whitespace().collect::<Vec<&str>>().join(" ");
        for argument in [
            "states: OPEN",
            "first: $first",
            "after: $after",
            "orderBy: { field: CREATED_AT, direction: DESC }",
            // Raised from five. A repository with more rules than the cap
            // would silently lose the one that governs a branch, and fall back
            // to treating every failing check as blocking.
            "branchProtectionRules(first: 25) { totalCount",
            // The count each nested connection is checked against. Asserted
            // per connection rather than by name, because `totalCount` appears
            // in six places and the field-name check cannot tell which of them
            // went missing.
            "reviewRequests(first: 10) { totalCount",
            "latestReviews(first: 10) { totalCount",
            "reviewThreads(first: 100) { totalCount",
            "contexts(first: 100) { totalCount",
        ] {
            assert!(
                normalised.contains(argument),
                "the query must ask for `{argument}`"
            );
        }
    }

    #[tokio::test]
    async fn every_page_is_fetched_and_the_cursor_is_carried_forward() {
        // A dropped page shrinks the population and reads as a repository that
        // merges faster than it does, so the count is held against the total
        // GitHub itself reported.
        let (outcome, sent) = fetch(vec![ok(PAGE_1), ok(PAGE_2)]).await;

        assert!(
            matches!(
                &outcome,
                Outcome::Fetched { collected, reported_total, viewer, rate_limit_remaining }
                    if *collected == TOTAL_OPEN
                        && *reported_total == Some(34)
                        && viewer == "user-09"
                        && rate_limit_remaining.is_some()
            ),
            "both pages answered, so every pull request must come back: {outcome:?}"
        );

        assert_eq!(sent.len(), 2, "two pages means two requests");
        assert_eq!(
            variable(&sent, 0, "after"),
            serde_json::Value::Null,
            "the first request asks for the start of the list"
        );
        assert_eq!(
            variable(&sent, 1, "after"),
            serde_json::Value::from(PAGE_1_END_CURSOR),
            "the second request must carry the cursor the first page ended on"
        );
        // The literal rather than `PAGE_SIZE`, which would compare the
        // constant with itself and hold whatever it was changed to.
        assert_eq!(variable(&sent, 0, "first"), serde_json::Value::from(50));
        assert_eq!(
            variable(&sent, 0, "owner"),
            serde_json::Value::from("example-org")
        );
        assert_eq!(
            variable(&sent, 0, "name"),
            serde_json::Value::from("example-repo")
        );
    }

    #[tokio::test]
    async fn every_request_carries_the_preview_header_and_the_credential() {
        // Without the preview media type `mergeStateStatus` does not resolve at
        // all, and GitHub refuses a request that does not name a user agent.
        let (_outcome, sent) = fetch(vec![ok(PAGE_1), ok(PAGE_2)]).await;
        assert_eq!(
            sent.len(),
            2,
            "the headers must be on every request, not just the first"
        );

        for request in &sent {
            let header = |name: &str| {
                request
                    .headers
                    .get(name)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .to_owned()
            };
            assert_eq!(header("accept"), MERGE_INFO_PREVIEW);
            assert_eq!(header("authorization"), format!("bearer {SECRET}"));
            assert!(
                header("user-agent").starts_with("prtrack-github/"),
                "GitHub refuses a request with no user agent"
            );
        }
    }

    #[tokio::test]
    async fn a_malformed_page_is_retried_rather_than_read_as_an_empty_repository() {
        // Measured against the live API, roughly one page in nine came back
        // without usable data. Taking that as "no pull requests" is the lie
        // this crate exists to avoid.
        let (outcome, sent) = fetch(vec![ok("{}"), ok(PAGE_1), ok(PAGE_2)]).await;

        assert!(
            matches!(&outcome, Outcome::Fetched { collected, .. } if *collected == TOTAL_OPEN),
            "the retry must recover the page: {outcome:?}"
        );
        assert_eq!(sent.len(), 3, "one wasted attempt and two good ones");
    }

    #[tokio::test]
    async fn a_transient_failure_that_never_clears_is_reported_after_the_budget() {
        let (outcome, sent) = fetch(vec![ResponseTemplate::new(502)]).await;

        assert!(
            matches!(
                &outcome,
                Outcome::Failed(GitHubError::GaveUp { attempts, source })
                    if *attempts == ATTEMPTS
                        && matches!(**source, GitHubError::UnexpectedStatus { status: 502 })
            ),
            "expected the budget to be spent and the last failure reported: {outcome:?}"
        );
        assert_eq!(
            sent.len(),
            ATTEMPTS as usize,
            "the whole budget must actually be spent"
        );
    }

    #[tokio::test]
    async fn a_rejected_token_is_not_retried() {
        // Retrying a 401 cannot help, and it turns one wrong answer into three.
        let (outcome, sent) = fetch(vec![ResponseTemplate::new(401)]).await;

        assert!(
            matches!(&outcome, Outcome::Failed(GitHubError::Unauthorized)),
            "expected an auth failure, got {outcome:?}"
        );
        assert_eq!(sent.len(), 1);
    }

    #[tokio::test]
    async fn an_exhausted_rate_limit_is_reported_rather_than_hammered() {
        let (outcome, sent) = fetch(vec![
            ResponseTemplate::new(403).insert_header("x-ratelimit-remaining", "0")
        ])
        .await;

        assert!(
            matches!(&outcome, Outcome::Failed(GitHubError::RateLimited)),
            "expected a rate limit, got {outcome:?}"
        );
        assert_eq!(
            sent.len(),
            1,
            "retrying inside a second is what earns a secondary limit"
        );
    }

    #[tokio::test]
    async fn a_forbidden_request_that_is_not_a_rate_limit_says_so() {
        // The two are a different instruction to the reader: one says "wait",
        // the other says "this token cannot do that".
        let (outcome, _sent) = fetch(vec![ResponseTemplate::new(403)]).await;

        assert!(
            matches!(&outcome, Outcome::Failed(GitHubError::Forbidden)),
            "expected a plain refusal, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn a_spent_rate_limit_is_reported_at_once_rather_than_retried() {
        // GitHub reports the *primary* GraphQL rate limit as HTTP 200 with an
        // error-only body, so the 403/429 guard never sees it. Read as a flake
        // it costs three requests against a budget that is already spent, and
        // reports as `GaveUp` — from which a shell cannot tell "wait for the
        // window" from "your query is broken".
        let body = r#"{"errors":[{"type":"RATE_LIMITED",
            "message":"API rate limit exceeded for user ID 1."}]}"#;
        let (outcome, sent) = fetch(vec![ok(body)]).await;

        assert!(
            matches!(&outcome, Outcome::Failed(GitHubError::RateLimited)),
            "expected the limit to be named, got {outcome:?}"
        );
        assert_eq!(sent.len(), 1, "a spent window does not reopen in 500ms");
    }

    #[tokio::test]
    async fn githubs_own_timeout_is_still_retried() {
        // The flake this crate was built around: roughly one page in nine came
        // back like this, and the next attempt was fine. It carries no `type`
        // at all, which is what separates it from a spent limit.
        let flake = r#"{"errors":[{"message":"Something went wrong while executing
            your query. This may be the result of a timeout, or it could be a
            GitHub bug."}]}"#;
        let (outcome, sent) = fetch(vec![ok(flake), ok(PAGE_1), ok(PAGE_2)]).await;

        assert!(
            matches!(&outcome, Outcome::Fetched { collected, .. } if *collected == TOTAL_OPEN),
            "the retry must still recover a timeout: {outcome:?}"
        );
        assert_eq!(sent.len(), 3);
    }

    #[tokio::test]
    async fn an_error_type_this_crate_does_not_know_is_still_retried() {
        // Retrying costs three quick attempts and then reports what GitHub
        // said. Refusing outright would turn a recoverable failure into a dead
        // poll, which is the more expensive mistake of the two.
        let body = r#"{"errors":[{"type":"SOMETHING_GITHUB_ADDED",
            "message":"who knows"}]}"#;
        let (outcome, sent) = fetch(vec![ok(body)]).await;

        assert!(
            matches!(
                &outcome,
                Outcome::Failed(GitHubError::GaveUp { source, .. })
                    if matches!(**source, GitHubError::GraphQl { .. })
            ),
            "expected the budget spent and GitHub's words reported, got {outcome:?}"
        );
        assert_eq!(sent.len(), ATTEMPTS as usize);
    }

    #[tokio::test]
    async fn a_field_an_error_nulled_is_refused_rather_than_read_as_absent() {
        // GraphQL nulls the field an error happened at and returns the rest of
        // the data. A nulled `statusCheckRollup` is then indistinguishable from
        // a pull request whose CI never ran — one is a transport failure, the
        // other is a fact about the repository, and they render identically as
        // "no checks". The path is the only thing that tells them apart.
        let mut body =
            serde_json::from_str::<serde_json::Value>(PAGE_1).unwrap_or(serde_json::Value::Null);
        if let Some(slot) = body.pointer_mut(
            "/data/repository/pullRequests/nodes/0/commits/nodes/0/commit/statusCheckRollup",
        ) {
            *slot = serde_json::Value::Null;
        }
        if let Some(envelope) = body.as_object_mut() {
            envelope.insert(
                "errors".to_owned(),
                serde_json::json!([{
                    "message": "Something went wrong while executing your query.",
                    "path": ["repository", "pullRequests", "nodes", 0, "commits",
                             "nodes", 0, "commit", "statusCheckRollup"]
                }]),
            );
        }
        let (outcome, _sent) = fetch(vec![ok(&body.to_string())]).await;

        assert!(
            matches!(
                &outcome,
                Outcome::Failed(GitHubError::GaveUp { source, .. })
                    if matches!(**source, GitHubError::Malformed(Malformed::ErrorNulledField { .. }))
            ),
            "expected the nulled field to be named, got {outcome:?}"
        );
    }

    /// The first recording with an `errors` array bolted on.
    fn page_1_with_error(error: &serde_json::Value) -> String {
        let mut body =
            serde_json::from_str::<serde_json::Value>(PAGE_1).unwrap_or(serde_json::Value::Null);
        if let Some(envelope) = body.as_object_mut() {
            envelope.insert("errors".to_owned(), serde_json::json!([error.clone()]));
        }
        body.to_string()
    }

    #[tokio::test]
    async fn an_error_that_nulled_nothing_does_not_cost_the_response() {
        // An error with no path nulled no field — GitHub's own timeout arrives
        // in this shape. The data around it is whole, and refusing it would
        // throw away a good fetch over a warning.
        let with_warning = page_1_with_error(&serde_json::json!({
            "message": "Something went wrong while executing your query."
        }));
        let (outcome, _sent) = fetch(vec![ok(&with_warning), ok(PAGE_2)]).await;

        assert!(
            matches!(&outcome, Outcome::Fetched { collected, .. } if *collected == TOTAL_OPEN),
            "a pathless error nulled nothing, so the data stands: {outcome:?}"
        );
    }

    #[tokio::test]
    async fn a_nulled_field_is_refused_wherever_in_the_response_it_is() {
        // Not only the check rollup. Any field an error nulled comes back
        // indistinguishable from that field being legitimately empty, and a
        // nulled author reads as a deleted account rather than as a failure.
        let nulled_author = page_1_with_error(&serde_json::json!({
            "message": "Something went wrong while executing your query.",
            "path": ["repository", "pullRequests", "nodes", 0, "author"]
        }));
        let (outcome, _sent) = fetch(vec![ok(&nulled_author)]).await;

        assert!(
            matches!(
                &outcome,
                Outcome::Failed(GitHubError::GaveUp { source, .. })
                    if matches!(
                        &**source,
                        GitHubError::Malformed(Malformed::ErrorNulledField { path })
                            if path == "repository.pullRequests.nodes.0.author"
                    )
            ),
            "expected the nulled author to be named, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn a_repository_that_is_not_visible_is_reported_and_not_retried() {
        // GitHub answers a missing repository and a private one the same way,
        // on purpose, so this cannot distinguish them either. Recorded from the
        // live API: this is the exact envelope a nonexistent owner returns, and
        // its error *does* carry a path — which must not be mistaken for a
        // field nulled out from under a response that otherwise worked.
        let body = r#"{"data":{"rateLimit":{"cost":1,"remaining":2538},
                       "viewer":{"login":"octocat"},"repository":null},
                       "errors":[{"type":"NOT_FOUND","path":["repository"],
                       "locations":[{"line":30,"column":3}],
                       "message":"Could not resolve to a Repository with the name 'x/y'."}]}"#;
        let (outcome, sent) = fetch(vec![ok(body)]).await;

        assert!(
            matches!(
                &outcome,
                Outcome::Failed(GitHubError::RepositoryUnavailable { repo })
                    if repo.to_string() == REPO
            ),
            "expected the repository to be named, got {outcome:?}"
        );
        assert_eq!(sent.len(), 1);
    }

    /// A recorded page with its reported total replaced.
    fn page_claiming_total(raw: &str, total: i64) -> String {
        let mut body =
            serde_json::from_str::<serde_json::Value>(raw).unwrap_or(serde_json::Value::Null);
        if let Some(slot) = body.pointer_mut("/data/repository/pullRequests/totalCount") {
            *slot = serde_json::Value::from(total);
        }
        body.to_string()
    }

    #[tokio::test]
    async fn pages_that_do_not_add_up_to_the_total_are_refused_and_read_again() {
        // `hasNextPage: false` is not the same as having everything. A pull
        // request created between two requests sorts ahead of the cursor and is
        // stepped straight over; GitHub still reports the list as ended and the
        // pair is quietly short. Nothing downstream can tell a short board from
        // a small repository, so the only defence is that the pages add up.
        let short = page_claiming_total(PAGE_2, 40);
        let (outcome, sent) = fetch(vec![
            ok(PAGE_1),
            ok(&short),
            ok(PAGE_1),
            ok(&short),
            ok(PAGE_1),
            ok(&short),
        ])
        .await;

        assert!(
            matches!(
                &outcome,
                Outcome::Failed(GitHubError::GaveUp { source, .. })
                    if matches!(**source, GitHubError::Malformed(Malformed::ListMoved { .. }))
            ),
            "expected the short pair to be refused, got {outcome:?}"
        );
        assert_eq!(
            sent.len(),
            6,
            "a list that moved is read again from the start, three times over"
        );
    }

    #[tokio::test]
    async fn a_total_that_changes_between_pages_is_refused() {
        // The other half of the same signal: if the total itself moved, the
        // population moved under the cursor even where the arithmetic happens
        // to work out.
        let first = page_claiming_total(PAGE_1, 38);
        let (outcome, _sent) = fetch(vec![ok(&first), ok(PAGE_2)]).await;

        assert!(
            matches!(
                &outcome,
                Outcome::Failed(GitHubError::GaveUp { source, .. })
                    if matches!(**source, GitHubError::Malformed(Malformed::ListMoved { .. }))
            ),
            "expected a moving total to be refused, got {outcome:?}"
        );
    }

    /// The recording with one JSON pointer replaced.
    fn page_1_patched(pointer: &str, value: serde_json::Value) -> String {
        let mut body =
            serde_json::from_str::<serde_json::Value>(PAGE_1).unwrap_or(serde_json::Value::Null);
        if let Some(slot) = body.pointer_mut(pointer) {
            *slot = value;
        }
        body.to_string()
    }

    #[tokio::test]
    async fn a_known_union_member_that_did_not_parse_is_refused() {
        // Not an unknown member — a known one whose fields changed shape. It
        // arrives in the same arm, and left alone every check run on the board
        // becomes a context with no readable state, which classifies as still
        // running. A repository with failing required checks would render as
        // one waiting for CI.
        let renamed = page_1_patched(
            "/data/repository/pullRequests/nodes/0/commits/nodes/0/commit/statusCheckRollup/contexts/nodes/0",
            serde_json::json!({
                "__typename": "CheckRun",
                "title": "gate",
                "conclusion": "FAILURE",
                "status": "COMPLETED",
                "startedAt": "2026-08-24T09:33:39Z"
            }),
        );
        let (outcome, _sent) = fetch(vec![ok(&renamed)]).await;

        assert!(
            matches!(
                &outcome,
                Outcome::Failed(GitHubError::GaveUp { source, .. })
                    if matches!(
                        &**source,
                        GitHubError::Malformed(Malformed::UnreadableMember { typename })
                            if typename == "CheckRun"
                    )
            ),
            "expected the contradiction to be named, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn a_reviewer_this_crate_names_that_did_not_parse_is_refused() {
        let renamed = page_1_patched(
            // Node 3 rather than 0: the first pull request in the recording has
            // no review requests, so patching it would assert nothing.
            "/data/repository/pullRequests/nodes/3/reviewRequests/nodes/0/requestedReviewer",
            serde_json::json!({"__typename": "User", "handle": "octocat"}),
        );
        let (outcome, _sent) = fetch(vec![ok(&renamed)]).await;

        assert!(
            matches!(
                &outcome,
                Outcome::Failed(GitHubError::GaveUp { source, .. })
                    if matches!(
                        &**source,
                        GitHubError::Malformed(Malformed::UnreadableMember { typename })
                            if typename == "User"
                    )
            ),
            "expected the contradiction to be named, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn a_union_member_nobody_has_heard_of_still_costs_nothing() {
        // The fallback has to keep working. A genuinely new member is not a
        // parse failure, and refusing it would defeat the reason it exists.
        let mannequin = page_1_patched(
            "/data/repository/pullRequests/nodes/0/reviewRequests/nodes/0/requestedReviewer",
            serde_json::json!({"__typename": "Mannequin", "login": "ghost"}),
        );
        let (outcome, _sent) = fetch(vec![ok(&mannequin), ok(PAGE_2)]).await;

        assert!(
            matches!(&outcome, Outcome::Fetched { collected, .. } if *collected == TOTAL_OPEN),
            "an unfamiliar member must survive, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn a_nested_list_that_came_back_short_is_refused() {
        // Every nested connection is capped, and until now only the outer one
        // was checked. A dropped review request loses somebody who is being
        // waited on; a dropped `latestReviews` entry loses a
        // `CHANGES_REQUESTED`; a dropped check attempt at a *required* check is
        // a false green. All of them are silent.
        let short = page_1_patched(
            "/data/repository/pullRequests/nodes/0/reviewRequests/totalCount",
            serde_json::Value::from(99),
        );
        let (outcome, _sent) = fetch(vec![ok(&short)]).await;

        assert!(
            matches!(
                &outcome,
                Outcome::Failed(GitHubError::GaveUp { source, .. })
                    if matches!(
                        **source,
                        GitHubError::Malformed(Malformed::Truncated {
                            what: NestedList::ReviewRequests,
                            ..
                        })
                    )
            ),
            "expected the short list to be named, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn a_protection_rule_list_that_came_back_short_is_refused() {
        // The worst of the five to lose. A missing rule is not a missing
        // detail: the pull requests it governs fall back to
        // `PolicyRules::unprotected`, where every failing check blocks, and the
        // board fills with reasons that are not real. False nudges are the
        // expensive direction.
        let short = page_1_patched(
            "/data/repository/branchProtectionRules/totalCount",
            serde_json::Value::from(3),
        );
        let (outcome, _sent) = fetch(vec![ok(&short)]).await;

        assert!(
            matches!(
                &outcome,
                Outcome::Failed(GitHubError::GaveUp { source, .. })
                    if matches!(
                        **source,
                        GitHubError::Malformed(Malformed::Truncated {
                            what: NestedList::ProtectionRules,
                            ..
                        })
                    )
            ),
            "expected the missing rule to be named, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn a_nested_list_that_reports_no_total_cannot_be_checked_and_is_refused() {
        // Same reasoning as the outer list: with no total there is nothing to
        // check against, and an absent number is not a clean bill of health.
        let mut body =
            serde_json::from_str::<serde_json::Value>(PAGE_1).unwrap_or(serde_json::Value::Null);
        if let Some(serde_json::Value::Object(contexts)) = body.pointer_mut(
            "/data/repository/pullRequests/nodes/0/commits/nodes/0/commit/statusCheckRollup/contexts",
        ) {
            contexts.remove("totalCount");
        }
        let (outcome, _sent) = fetch(vec![ok(&body.to_string())]).await;

        assert!(
            matches!(
                &outcome,
                Outcome::Failed(GitHubError::GaveUp { source, .. })
                    if matches!(
                        **source,
                        GitHubError::Malformed(Malformed::Uncountable {
                            what: NestedList::CheckContexts
                        })
                    )
            ),
            "expected an uncountable list to be refused, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn a_page_with_no_page_info_is_not_taken_for_the_last_one() {
        // `Malformed::NoPageInfo`'s own documentation says treating a missing
        // `pageInfo` as the end of the list turns a truncated list into a
        // complete-looking one — and no test omitted it, so the guard that
        // says so was held up by nothing.
        let mut body =
            serde_json::from_str::<serde_json::Value>(PAGE_1).unwrap_or(serde_json::Value::Null);
        if let Some(serde_json::Value::Object(connection)) =
            body.pointer_mut("/data/repository/pullRequests")
        {
            connection.remove("pageInfo");
        }
        let (outcome, _sent) = fetch(vec![ok(&body.to_string())]).await;

        assert!(
            matches!(
                &outcome,
                Outcome::Failed(GitHubError::GaveUp { source, .. })
                    if matches!(**source, GitHubError::Malformed(Malformed::NoPageInfo))
            ),
            "expected a page with no pageInfo to be refused, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn the_backoff_between_attempts_at_one_page_is_actually_waited_out() {
        // `RetryBudget::schedule` is well tested in isolation and the attempt
        // *count* is pinned, but nothing connected the two: deleting the sleep
        // left every test passing, and a retry loop that does not back off
        // hammers an API that is already struggling.
        //
        // # Why this is not on a virtual clock
        //
        // `#[tokio::test(start_paused = true)]` is the obvious way to make this
        // instant and exact, and `test-util` is enabled, so it is available.
        // It does not work here, and the reason is worth writing down: paused
        // time auto-advances to the next timer whenever the runtime has nothing
        // runnable, and a test that talks to a mock server over a real socket
        // spends most of its time waiting on exactly that. The nearest timer is
        // then the client's own per-request timeout, so the clock jumps to it
        // and the request times out having never been slow:
        //
        //     GaveUp { source: Transport(reqwest::Error { kind: Request,
        //                                                 source: TimedOut }) }
        //
        // Raising the timeout does not help — the clock jumps to wherever it
        // is. Paused time and real I/O are incompatible, not merely awkward.
        //
        // So: the real clock, and a **lower bound**. A loaded machine can only
        // make elapsed time larger, so this cannot flake in the direction that
        // matters, while a deleted sleep finishes two orders of magnitude
        // under it. The cost is roughly 300ms per test.
        let first_backoff = Duration::from_millis(100);
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::method("POST"))
            .respond_with(ResponseTemplate::new(502))
            .mount(&server)
            .await;

        let client = client_for(&server).map(|client| {
            client.with_retry_budget(RetryBudget::new(
                NonZeroU32::new(ATTEMPTS).unwrap_or(NonZeroU32::MIN),
                first_backoff,
            ))
        });

        let started = std::time::Instant::now();
        let outcome = run(client).await;
        let elapsed = started.elapsed();
        drop(server);

        assert!(
            matches!(&outcome, Outcome::Failed(GitHubError::GaveUp { attempts, .. }) if *attempts == ATTEMPTS),
            "the budget must still be spent: {outcome:?}"
        );
        // Three attempts wait nothing, then 100ms, then 200ms.
        let owed = first_backoff * 3;
        assert!(
            elapsed >= owed,
            "the schedule prescribes {owed:?} of waiting and only {elapsed:?} passed"
        );
    }

    #[tokio::test]
    async fn the_backoff_before_reading_the_whole_list_again_is_waited_out_too() {
        // There are two retry loops — one over the attempts at a single page,
        // one over whole passes when the list moved — and each has its own
        // sleep. The first test above cannot reach this one, because a page
        // that fails permanently never reaches a second pass. Deleting *this*
        // sleep left everything green, which is how it was found.
        //
        // Real clock, for the reason given on that test. This is the one that
        // demonstrated it: under `start_paused` it fails with a spurious
        // `TimedOut` before it ever reaches the assertion.
        let first_backoff = Duration::from_millis(100);
        let short = page_claiming_total(PAGE_2, 40);
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::method("POST"))
            .respond_with(Script::of(vec![
                ok(PAGE_1),
                ok(&short),
                ok(PAGE_1),
                ok(&short),
                ok(PAGE_1),
                ok(&short),
            ]))
            .mount(&server)
            .await;

        let client = client_for(&server).map(|client| {
            client.with_retry_budget(RetryBudget::new(
                NonZeroU32::new(ATTEMPTS).unwrap_or(NonZeroU32::MIN),
                first_backoff,
            ))
        });

        let started = std::time::Instant::now();
        let outcome = run(client).await;
        let elapsed = started.elapsed();
        drop(server);

        assert!(
            matches!(
                &outcome,
                Outcome::Failed(GitHubError::GaveUp { source, .. })
                    if matches!(**source, GitHubError::Malformed(Malformed::ListMoved { .. }))
            ),
            "the list must still be refused: {outcome:?}"
        );
        // Three passes wait nothing, then 100ms, then 200ms.
        let owed = first_backoff * 3;
        assert!(
            elapsed >= owed,
            "the schedule prescribes {owed:?} between passes and only {elapsed:?} passed"
        );
    }

    #[tokio::test]
    async fn a_page_that_reports_no_total_at_all_is_refused() {
        // The arithmetic is the only thing standing between a short board and a
        // small repository. With no total there is no arithmetic, so there is
        // nothing to check against and the safe reading is to refuse — not to
        // treat an absent number as agreement.
        // The second recording, because it is the one that ends the list.
        let mut body =
            serde_json::from_str::<serde_json::Value>(PAGE_2).unwrap_or(serde_json::Value::Null);
        if let Some(serde_json::Value::Object(connection)) =
            body.pointer_mut("/data/repository/pullRequests")
        {
            connection.remove("totalCount");
        }
        let (outcome, _sent) = fetch(vec![ok(&body.to_string())]).await;

        assert!(
            matches!(
                &outcome,
                Outcome::Failed(GitHubError::GaveUp { source, .. })
                    if matches!(**source, GitHubError::Malformed(Malformed::ListMoved { .. }))
            ),
            "expected a page with no total to be refused, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn a_page_promising_another_without_a_cursor_is_refused() {
        // Reading that as the end of the list would report a truncated board as
        // a complete one.
        let body = page_1_with(&serde_json::json!({
            "totalCount": 40,
            "pageInfo": {"hasNextPage": true, "endCursor": null},
            "nodes": []
        }));
        let (outcome, _sent) = fetch(vec![ok(&body)]).await;

        assert!(
            matches!(
                &outcome,
                Outcome::Failed(GitHubError::GaveUp { source, .. })
                    if matches!(**source, GitHubError::Malformed(Malformed::NoCursor))
            ),
            "expected a refusal to guess, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn a_null_node_is_refused_rather_than_silently_shrinking_the_list() {
        let body = page_1_with(&serde_json::json!({
            "totalCount": 40,
            "pageInfo": {"hasNextPage": false, "endCursor": null},
            "nodes": [null]
        }));
        let (outcome, _sent) = fetch(vec![ok(&body)]).await;

        assert!(
            matches!(
                &outcome,
                Outcome::Failed(GitHubError::GaveUp { source, .. })
                    if matches!(**source, GitHubError::Malformed(Malformed::NullNode))
            ),
            "expected the hole to be reported, got {outcome:?}"
        );
    }

    #[tokio::test]
    async fn a_page_with_no_nodes_list_is_not_read_as_an_empty_one() {
        let body = page_1_with(&serde_json::json!({
            "totalCount": 40,
            "pageInfo": {"hasNextPage": false, "endCursor": null}
        }));
        let (outcome, _sent) = fetch(vec![ok(&body)]).await;

        assert!(
            matches!(
                &outcome,
                Outcome::Failed(GitHubError::GaveUp { source, .. })
                    if matches!(**source, GitHubError::Malformed(Malformed::NoNodes))
            ),
            "expected a missing list to be distinguished from an empty one: {outcome:?}"
        );
    }

    #[tokio::test]
    async fn a_repository_with_nothing_open_succeeds_with_an_empty_list() {
        // An empty board and a broken fetch must never look alike.
        let body = page_1_with(&serde_json::json!({
            "totalCount": 0,
            "pageInfo": {"hasNextPage": false, "endCursor": null},
            "nodes": []
        }));
        let (outcome, sent) = fetch(vec![ok(&body)]).await;

        assert!(
            matches!(
                &outcome,
                Outcome::Fetched { collected, reported_total, .. }
                    if *collected == 0 && *reported_total == Some(0)
            ),
            "an empty repository is a success, not a failure: {outcome:?}"
        );
        assert_eq!(sent.len(), 1);
    }

    #[tokio::test]
    async fn pagination_stops_instead_of_following_a_cursor_forever() {
        // A page that always says another follows would otherwise spin against
        // a poll loop until someone noticed.
        let body = page_1_with(&serde_json::json!({
            "totalCount": 40,
            "pageInfo": {"hasNextPage": true, "endCursor": "always-more"},
            "nodes": []
        }));
        let (outcome, sent) = fetch(vec![ok(&body)]).await;

        assert!(
            matches!(&outcome, Outcome::Failed(GitHubError::TooManyPages { .. })),
            "expected the cap to stop it, got {outcome:?}"
        );
        assert_eq!(sent.len(), MAX_PAGES, "the cap must be what stopped it");
    }

    #[tokio::test]
    async fn a_request_that_hangs_is_abandoned_rather_than_stalling_the_loop() {
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(PAGE_1.to_owned(), "application/json")
                    .set_delay(Duration::from_secs(30)),
            )
            .mount(&server)
            .await;

        let client =
            client_for(&server).map(|client| client.with_timeout(Duration::from_millis(50)));
        let outcome = run(client).await;
        drop(server);

        assert!(
            matches!(
                &outcome,
                Outcome::Failed(GitHubError::GaveUp { source, .. })
                    if matches!(**source, GitHubError::Transport(_))
            ),
            "expected the request to be abandoned well before its 30 seconds: {outcome:?}"
        );
    }

    #[tokio::test]
    async fn the_hand_written_debug_impls_render_exactly_what_they_promise() {
        // Both can be replaced with leaking ones and nothing else fails,
        // because what actually keeps the token out of a log is
        // `HeaderValue::set_sensitive`. That makes the hand-written impls
        // unenforced defence, which a refactor deletes without noticing. Pin
        // the rendering so deleting one is a test failure rather than a
        // silent loss of the second layer.
        let server = MockServer::start().await;
        let rendered_token = Token::parse(SECRET)
            .map(|token| format!("{token:?}"))
            .unwrap_or_default();
        assert_eq!(
            rendered_token, "Token { .. }",
            "a token prints as its type and nothing else"
        );

        let rendered_client = client_for(&server)
            .map(|client| format!("{client:?}"))
            .unwrap_or_default();
        drop(server);
        for named in ["GitHubClient {", "endpoint", "retry", "timeout"] {
            assert!(
                rendered_client.contains(named),
                "the client should still say {named:?}: {rendered_client}"
            );
        }
        // What a leaking impl would print: `reqwest::Client`'s own `Debug`
        // renders its accepted encodings, its proxies and its default headers,
        // and the default headers are where the credential lives.
        for leaked in [
            "accepts",
            "proxies",
            "default_headers",
            "authorization",
            "Sensitive",
        ] {
            assert!(
                !rendered_client
                    .to_lowercase()
                    .contains(&leaked.to_lowercase()),
                "the HTTP client must not be printed; it holds the credential \
                 in its default headers: {rendered_client}"
            );
        }
    }

    #[tokio::test]
    async fn nothing_the_client_can_print_carries_the_credential() {
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::method("POST"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let rendered = format!("{client:?}");
        assert!(
            !rendered.contains(SECRET),
            "the client's own Debug leaked it: {rendered}"
        );

        let outcome = run(client).await;
        drop(server);
        assert!(
            matches!(&outcome, Outcome::Failed(_)),
            "a 401 must fail or this test proves nothing: {outcome:?}"
        );
        assert!(
            !format!("{outcome:?}").contains(SECRET),
            "the error chain leaked it: {outcome:?}"
        );
    }

    #[tokio::test]
    async fn an_endpoint_that_could_carry_a_credential_into_a_log_is_refused() {
        let server = MockServer::start().await;
        let refused = [
            format!("https://user:{SECRET}@example.invalid/graphql"),
            // Userinfo with no password at all, which is the shape a pasted
            // token actually takes. Deleting the username check leaves every
            // other test in this crate passing, and `reqwest` appends
            // `for url (...)` to its own message, so this reaches a log.
            format!("https://{SECRET}@example.invalid/graphql"),
            "ftp://example.invalid/graphql".to_owned(),
            "not a url".to_owned(),
        ];

        for endpoint in refused {
            let outcome = client_for(&server)
                .and_then(|client| client.with_endpoint(&endpoint))
                .err();
            assert!(
                matches!(outcome, Some(GitHubError::UnusableEndpoint)),
                "expected {endpoint:?} to be refused, got {outcome:?}"
            );
            assert!(
                !format!("{outcome:?}").contains(SECRET),
                "the refusal must not echo the URL it refused"
            );
        }
    }
}
