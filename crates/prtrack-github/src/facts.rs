//! Turning a fetched response into the facts the decision core reasons about.
//!
//! This is the boundary. Above it every value is a domain type with its
//! invariants checked; below it everything is whatever GitHub sent. The
//! conversion happens once, here, so no consumer has to re-validate and none
//! can forget to.
//!
//! # What it refuses
//!
//! A value GitHub sent that no domain type accepts fails the whole conversion
//! rather than dropping one pull request. A board short by a row reads as a
//! repository with less open work than it has, and nothing downstream can tell
//! the difference. None of these can fire against the live API today; if one
//! ever does it should be loud rather than quiet.

use chrono::{DateTime, Utc};
use prtrack_core::{
    BranchName, Check, CheckName, CheckState, DraftState, HeadSha, Login, MergeState,
    MergeableState, PolicyRules, PrFacts, PrNumber, PrTitle, PrUrl, RepoRef, ReviewDecision,
    Reviewer, TeamSlug,
};

use crate::client::OpenPullRequests;
use crate::error::ConversionError;
use crate::policy;
use crate::wire;

/// Every open pull request as domain facts, each with the policy it is judged
/// against.
///
/// The policy is carried per pull request rather than once for the board,
/// because that is the question it answers: a stacked pull request aimed at
/// another feature branch is governed by no rule even where its neighbours are
/// governed by a strict one. `BoardRow::reduce` takes a policy per row for the
/// same reason.
#[derive(Debug, Clone)]
pub struct RepositoryFacts {
    viewer: Login,
    default_branch: Option<BranchName>,
    default_branch_rule: PolicyRules,
    pull_requests: Vec<(PrFacts, PolicyRules)>,
}

impl RepositoryFacts {
    /// The account the token authenticates as, which decides whose work is
    /// whose.
    #[must_use]
    pub const fn viewer(&self) -> &Login {
        &self.viewer
    }

    /// The repository's default branch.
    ///
    /// `None` only for a repository with no commits, which therefore has no
    /// open pull requests either — so a caller with rows to render always has
    /// a name here. Absent rather than fabricated: a board that names the
    /// wrong branch in its policy line is worse than one that admits it does
    /// not know, because nothing about the wrong name looks wrong.
    #[must_use]
    pub const fn default_branch(&self) -> Option<&BranchName> {
        self.default_branch.as_ref()
    }

    /// The rule governing the default branch, which is what a policy line
    /// describes.
    ///
    /// Not an `Option`: [`PolicyRules::unprotected`] is a real value meaning
    /// "no rule speaks for this branch, so every failing check blocks", and an
    /// unprotected branch is a state rather than a missing answer. This is
    /// also what a repository with no default branch reports, for the same
    /// reason — silence about the policy, read the safe way.
    #[must_use]
    pub const fn default_branch_rule(&self) -> &PolicyRules {
        &self.default_branch_rule
    }

    /// Every open pull request, each beside the policy that judges it.
    #[must_use]
    pub fn pull_requests(&self) -> &[(PrFacts, PolicyRules)] {
        &self.pull_requests
    }

    /// The same, owned, for handing to `BoardRow::reduce` without cloning.
    ///
    /// Carries the repository-level facts too, because a renderer needs them
    /// beside the rows and reading them through the accessors would borrow a
    /// value this consumes.
    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        Login,
        Option<BranchName>,
        PolicyRules,
        Vec<(PrFacts, PolicyRules)>,
    ) {
        (
            self.viewer,
            self.default_branch,
            self.default_branch_rule,
            self.pull_requests,
        )
    }
}

impl TryFrom<OpenPullRequests> for RepositoryFacts {
    type Error = ConversionError;

    /// Converts a fetched response into domain facts.
    ///
    /// # Errors
    ///
    /// See [`ConversionError`]. Every variant names the pull request or rule
    /// the offending value came from.
    fn try_from(open: OpenPullRequests) -> Result<Self, Self::Error> {
        let viewer = Login::parse(&open.first_page.viewer.login)
            .map_err(|source| ConversionError::Viewer { source })?;

        // A response with no repository cannot happen through the client, which
        // refuses such a page. If one ever arrived, no rule is readable, and
        // that is exactly what `PolicyRules::unprotected` means: silence about
        // the policy, under which every failing check blocks.
        let rules: Vec<wire::BranchProtectionRule> = open
            .first_page
            .repository
            .iter()
            .flat_map(|repository| repository.branch_protection_rules.nodes.iter())
            .flatten()
            .flatten()
            .cloned()
            .collect();

        let default_branch = open
            .first_page
            .repository
            .as_ref()
            .and_then(|repository| repository.default_branch_ref.as_ref())
            .map(|branch| BranchName::new(&branch.name));

        // Resolved through the same matcher every row uses, so the policy line
        // and the rows can never describe different rules for one branch.
        let default_branch_rule = match default_branch.as_ref() {
            Some(branch) => policy::rules_for(branch.as_str(), &rules)?,
            None => PolicyRules::unprotected(),
        };

        let pull_requests = open
            .pull_requests
            .iter()
            .map(|pull_request| facts_of(&open.repo, &rules, pull_request))
            .collect::<Result<Vec<_>, ConversionError>>()?;

        Ok(Self {
            viewer,
            default_branch,
            default_branch_rule,
            pull_requests,
        })
    }
}

/// One pull request, as facts.
fn facts_of(
    repo: &RepoRef,
    rules: &[wire::BranchProtectionRule],
    pull_request: &wire::PullRequest,
) -> Result<(PrFacts, PolicyRules), ConversionError> {
    let number = pull_request.number;
    let refuse = |source| ConversionError::PullRequest { number, source };

    let commit = pull_request
        .commits
        .nodes
        .iter()
        .flatten()
        .flatten()
        .map(|node| &node.commit)
        .next()
        .ok_or(ConversionError::NoCommit { number })?;

    let facts = PrFacts {
        repo: repo.clone(),
        number: PrNumber::try_from(number).map_err(refuse)?,
        url: PrUrl::new(&pull_request.url),
        title: PrTitle::new(&pull_request.title),
        author: pull_request
            .author
            .as_ref()
            .map(|actor| Login::parse(&actor.login))
            .transpose()
            .map_err(refuse)?,
        draft_state: if pull_request.is_draft {
            DraftState::Draft
        } else {
            DraftState::ReadyForReview
        },
        head: HeadSha::parse(&pull_request.head_ref_oid).map_err(refuse)?,
        base_ref: BranchName::new(&pull_request.base_ref_name),
        review_decision: pull_request
            .review_decision
            .as_ref()
            .map(review_decision_of),
        mergeable: mergeable_of(&pull_request.mergeable),
        merge_state: merge_state_of(&pull_request.merge_state_status),
        requested_reviewers: reviewers_of(pull_request).map_err(refuse)?,
        changes_requested_by: changes_requested_by(pull_request).map_err(refuse)?,
        unresolved_threads: unresolved_threads(pull_request),
        comments: u32::try_from(pull_request.comments.total_count).map_err(|_source| {
            ConversionError::NegativeCount {
                number,
                count: pull_request.comments.total_count,
            }
        })?,
        checks: checks_of(number, commit)?,
        last_push: instant_of(number, &commit.committed_date)?,
    };

    let policy = policy::rules_for(&pull_request.base_ref_name, rules)?;
    Ok((facts, policy))
}

/// Everyone still being waited on for a review.
fn reviewers_of(
    pull_request: &wire::PullRequest,
) -> Result<Vec<Reviewer>, prtrack_core::InvalidValue> {
    pull_request
        .review_requests
        .nodes
        .iter()
        .flatten()
        .flatten()
        .map(|request| match &request.requested_reviewer {
            Some(wire::RequestedReviewer::User { login }) => {
                Login::parse(login).map(Reviewer::User)
            }
            Some(wire::RequestedReviewer::Team { slug }) => Ok(Reviewer::Team(TeamSlug::new(slug))),
            Some(wire::RequestedReviewer::Other(node)) => Ok(Reviewer::unknown(&node.typename)),
            // A request GitHub would not resolve — a team in another
            // organisation, most often. Somebody was asked, and dropping the
            // request would render the pull request as waiting on nobody.
            None => Ok(Reviewer::unknown("")),
        })
        .collect()
}

/// The reviewers whose latest word was a request for changes.
fn changes_requested_by(
    pull_request: &wire::PullRequest,
) -> Result<Vec<Login>, prtrack_core::InvalidValue> {
    pull_request
        .latest_reviews
        .nodes
        .iter()
        .flatten()
        .flatten()
        .filter(|review| matches!(review.state, wire::ReviewState::ChangesRequested))
        // A review by an account since deleted still counts against the pull
        // request through `review_decision`, but it cannot be named here.
        .filter_map(|review| review.author.as_ref())
        .map(|actor| Login::parse(&actor.login))
        .collect()
}

/// How many review threads are still open.
///
/// The query asks for the first hundred threads, so a pull request with more
/// than that is undercounted.
fn unresolved_threads(pull_request: &wire::PullRequest) -> u32 {
    let open = pull_request
        .review_threads
        .nodes
        .iter()
        .flatten()
        .flatten()
        .filter(|thread| !thread.is_resolved)
        .count();
    u32::try_from(open).unwrap_or(u32::MAX)
}

/// Every attempt reported on the head commit, unmerged and unjudged.
fn checks_of(number: i64, commit: &wire::Commit) -> Result<Vec<Check>, ConversionError> {
    let refuse = |source| ConversionError::PullRequest { number, source };

    commit
        .status_check_rollup
        .iter()
        .flat_map(|rollup| rollup.contexts.nodes.iter().flatten().flatten())
        .map(|context| match context {
            wire::CheckContext::CheckRun {
                name,
                conclusion,
                status,
                started_at,
            } => Ok(Check::new(
                CheckName::parse(name).map_err(refuse)?,
                check_state_of(conclusion.as_ref(), status),
                started_at
                    .as_deref()
                    .map(|at| instant_of(number, at))
                    .transpose()?,
            )),
            wire::CheckContext::StatusContext {
                context,
                state,
                created_at,
            } => Ok(Check::new(
                CheckName::parse(context).map_err(refuse)?,
                CheckState::parse(&status_state_text(state)),
                Some(instant_of(number, created_at)?),
            )),
            // A member of the union this crate does not know. Recorded rather
            // than dropped, and as a state nobody can read, so it counts as
            // still running: a context we cannot interpret has not passed, and
            // dropping it would hide whatever it was about to say.
            wire::CheckContext::Other(node) => Ok(Check::new(
                CheckName::parse(&node.typename).map_err(refuse)?,
                CheckState::parse(&node.typename),
                None,
            )),
        })
        .collect()
}

/// What a check run reported.
///
/// **The conclusion is read first, and the status only when there is none.**
/// This is the trap the whole module is most exposed to: `COMPLETED` was 1428
/// of 1430 check runs in the recorded response, it is a perfectly good
/// `CheckStatusState`, and it means nothing at all in the core's vocabulary. A
/// naive reading of `status` therefore turns every finished check into an
/// unrecognised state, which classifies as *still running*, and the board shows
/// an entire repository waiting on CI that finished hours ago.
///
fn check_state_of(
    conclusion: Option<&wire::CheckConclusionState>,
    status: &wire::CheckStatusState,
) -> CheckState {
    conclusion.map_or_else(|| status_text_state(status), conclusion_state)
}

/// The core's word for a finished run's verdict.
fn conclusion_state(conclusion: &wire::CheckConclusionState) -> CheckState {
    match conclusion {
        wire::CheckConclusionState::ActionRequired => CheckState::ActionRequired,
        wire::CheckConclusionState::TimedOut => CheckState::TimedOut,
        wire::CheckConclusionState::Cancelled => CheckState::Cancelled,
        wire::CheckConclusionState::Failure => CheckState::Failure,
        wire::CheckConclusionState::Success => CheckState::Success,
        wire::CheckConclusionState::Neutral => CheckState::Neutral,
        wire::CheckConclusionState::Skipped => CheckState::Skipped,
        wire::CheckConclusionState::StartupFailure => CheckState::StartupFailure,
        wire::CheckConclusionState::Stale => CheckState::Stale,
        wire::CheckConclusionState::Unknown(reported) => CheckState::parse(reported),
    }
}

/// The core's word for how far along an unfinished run is.
fn status_text_state(status: &wire::CheckStatusState) -> CheckState {
    match status {
        wire::CheckStatusState::Queued => CheckState::Queued,
        wire::CheckStatusState::InProgress => CheckState::InProgress,
        wire::CheckStatusState::Waiting => CheckState::Waiting,
        wire::CheckStatusState::Pending => CheckState::Pending,
        // The core names neither. `REQUESTED` is a run not yet queued, and
        // `COMPLETED` here means a finished run that reported no conclusion,
        // which contradicts itself. Both go through `parse`, which keeps the
        // text legible and counts them as unfinished — the honest reading of a
        // state we cannot interpret.
        wire::CheckStatusState::Requested => CheckState::parse("REQUESTED"),
        wire::CheckStatusState::Completed => CheckState::parse("COMPLETED"),
        wire::CheckStatusState::Unknown(reported) => CheckState::parse(reported),
    }
}

/// The wire spelling of a status context's state, for the core to read.
fn status_state_text(state: &wire::StatusState) -> String {
    match state {
        wire::StatusState::Expected => "EXPECTED".to_owned(),
        wire::StatusState::Error => "ERROR".to_owned(),
        wire::StatusState::Failure => "FAILURE".to_owned(),
        wire::StatusState::Pending => "PENDING".to_owned(),
        wire::StatusState::Success => "SUCCESS".to_owned(),
        wire::StatusState::Unknown(reported) => reported.clone(),
    }
}

/// The core's word for whether git can merge the branch.
fn mergeable_of(state: &wire::MergeableState) -> MergeableState {
    match state {
        wire::MergeableState::Mergeable => MergeableState::Mergeable,
        wire::MergeableState::Conflicting => MergeableState::Conflicting,
        wire::MergeableState::Computing => MergeableState::Computing,
        wire::MergeableState::Unknown(reported) => MergeableState::parse(reported),
    }
}

/// The core's word for GitHub's verdict on the merge button.
fn merge_state_of(status: &wire::MergeStateStatus) -> MergeState {
    match status {
        wire::MergeStateStatus::Behind => MergeState::Behind,
        wire::MergeStateStatus::Blocked => MergeState::Blocked,
        wire::MergeStateStatus::Clean => MergeState::Clean,
        wire::MergeStateStatus::Dirty => MergeState::Dirty,
        wire::MergeStateStatus::Draft => MergeState::Draft,
        wire::MergeStateStatus::HasHooks => MergeState::HasHooks,
        wire::MergeStateStatus::Undetermined => MergeState::Undetermined,
        wire::MergeStateStatus::Unstable => MergeState::Unstable,
        wire::MergeStateStatus::Unknown(reported) => MergeState::parse(reported),
    }
}

/// The core's word for where the review process stands.
fn review_decision_of(decision: &wire::ReviewDecision) -> ReviewDecision {
    match decision {
        wire::ReviewDecision::Approved => ReviewDecision::Approved,
        wire::ReviewDecision::ChangesRequested => ReviewDecision::ChangesRequested,
        wire::ReviewDecision::ReviewRequired => ReviewDecision::ReviewRequired,
        wire::ReviewDecision::Unknown(reported) => ReviewDecision::parse(reported),
    }
}

/// Reads one of GitHub's timestamps.
///
/// A timestamp that will not parse fails the conversion rather than becoming
/// `None`. An attempt with no instant loses the recency tie-break, so a
/// timestamp quietly dropped could let a superseded failure speak for a check
/// that has since been re-run green.
fn instant_of(number: i64, raw: &str) -> Result<DateTime<Utc>, ConversionError> {
    DateTime::parse_from_rfc3339(raw)
        .map(|at| at.with_timezone(&Utc))
        .map_err(|source| ConversionError::Timestamp { number, source })
}

#[cfg(test)]
mod tests {
    use super::RepositoryFacts;
    use crate::client::OpenPullRequests;
    use crate::error::ConversionError;
    use crate::wire::{self, PAGE_1, PAGE_2, TOTAL_OPEN};
    use prtrack_core::{
        BlockingChecks, Board, BoardRow, BranchName, CheckState, DraftState, MergeState,
        MergeableState, RepoRef,
    };

    /// The repository the recordings are of, after redaction.
    const REPO: &str = "example-org/example-repo";

    /// One recorded page, read back.
    fn page(raw: &str) -> Option<wire::Query> {
        serde_json::from_str::<wire::Envelope>(raw).ok()?.data
    }

    /// The recorded response, assembled the way the client assembles it.
    ///
    /// Built straight from the fixtures rather than through a mock server: the
    /// client's own tests already cover the fetching, and what is under test
    /// here is the conversion. `None` if a fixture will not parse, which every
    /// assertion below rejects rather than passing over.
    fn recorded() -> Option<OpenPullRequests> {
        let first_page = page(PAGE_1)?;
        let second_page = page(PAGE_2)?;
        let pull_requests: Vec<wire::PullRequest> = [&first_page, &second_page]
            .into_iter()
            .filter_map(|page| page.repository.as_ref())
            .filter_map(|repository| repository.pull_requests.nodes.as_ref())
            .flatten()
            .flatten()
            .cloned()
            .collect();

        Some(OpenPullRequests {
            repo: RepoRef::parse(REPO).ok()?,
            first_page,
            pull_requests,
        })
    }

    /// The recorded response with every pull request passed through `edit`.
    fn recorded_with(edit: impl Fn(&mut wire::PullRequest)) -> Option<OpenPullRequests> {
        let mut open = recorded()?;
        for pull_request in &mut open.pull_requests {
            edit(pull_request);
        }
        Some(open)
    }

    /// The converted recording, or `None` if it did not convert.
    fn converted() -> Option<RepositoryFacts> {
        recorded().and_then(|open| RepositoryFacts::try_from(open).ok())
    }

    /// The recording, edited by `edit`, converted.
    fn converted_with(edit: impl Fn(&mut wire::PullRequest)) -> Option<RepositoryFacts> {
        recorded_with(edit).and_then(|open| RepositoryFacts::try_from(open).ok())
    }

    /// What converting the recording, edited by `edit`, failed with.
    fn refusal(edit: impl Fn(&mut wire::PullRequest)) -> Option<ConversionError> {
        recorded_with(edit).and_then(|open| RepositoryFacts::try_from(open).err())
    }

    #[test]
    fn every_recorded_pull_request_becomes_facts() {
        let facts = converted();
        assert_eq!(
            facts.as_ref().map(|f| f.pull_requests().len()),
            Some(TOTAL_OPEN),
            "a conversion that drops a row shortens the board silently"
        );
        assert_eq!(
            facts.as_ref().map(|f| f.viewer().as_str().to_owned()),
            Some("user-09".to_owned())
        );
    }

    #[test]
    fn a_pull_request_carries_the_identity_the_board_leads_with() {
        let facts = converted();
        let first = facts
            .as_ref()
            .and_then(|f| f.pull_requests().first())
            .map(|(facts, _)| facts);
        assert!(
            first.is_some(),
            "the recording converts into at least one pull request"
        );

        assert_eq!(first.map(|f| f.number.get()), Some(4440));
        assert_eq!(first.map(|f| f.repo.to_string()), Some(REPO.to_owned()));
        assert_eq!(first.map(|f| f.head.as_str().len()), Some(40));
        assert_eq!(first.map(|f| f.base_ref.as_str()), Some("main"));
        assert_eq!(
            first.map(|f| !f.url.as_str().is_empty() && !f.title.as_str().is_empty()),
            Some(true)
        );
        assert_eq!(
            first.map(|f| f.author.is_some()),
            Some(true),
            "an open pull request has an author"
        );
        assert_eq!(
            first.map(|f| f.draft_state),
            Some(DraftState::ReadyForReview)
        );
    }

    #[test]
    fn a_finished_check_reports_what_it_decided_not_that_it_finished() {
        // The trap this module is most exposed to. `COMPLETED` was 1428 of 1430
        // check runs in the recording; it is a real `CheckStatusState` and means
        // nothing in the core's vocabulary. Read naively, every finished check
        // becomes an unrecognised state, which classifies as *still running* —
        // and the board shows the whole repository waiting on CI that finished
        // hours ago.
        let facts = converted();
        let states: Vec<CheckState> = facts
            .iter()
            .flat_map(RepositoryFacts::pull_requests)
            .flat_map(|(facts, _)| facts.checks.iter())
            .map(|check| check.state().clone())
            .collect();

        assert!(
            states.len() > 1000,
            "the recording holds over a thousand attempts, got {}",
            states.len()
        );
        let unreadable: Vec<&CheckState> = states
            .iter()
            .filter(|state| matches!(state, CheckState::Unknown { .. }))
            .collect();
        assert!(
            unreadable.is_empty(),
            "no recorded check should be unreadable, got {unreadable:?}"
        );
        assert!(
            states.contains(&CheckState::Success),
            "the recording is mostly passing checks, so some must read as passed"
        );
    }

    #[test]
    fn a_check_still_running_reports_how_far_along_it_is() {
        // The other side of the same rule: with no conclusion yet, the status is
        // the only word there is.
        let facts = converted();
        let running = facts
            .iter()
            .flat_map(RepositoryFacts::pull_requests)
            .flat_map(|(facts, _)| facts.checks.iter())
            .filter(|check| *check.state() == CheckState::InProgress)
            .count();
        assert!(
            running > 0,
            "the recording holds a run still in progress, whose conclusion is null"
        );
    }

    #[test]
    fn every_attempt_keeps_the_instant_that_orders_it() {
        // Without it a repeated check name cannot be reduced to its newest
        // attempt, which is how a superseded failure speaks for a check that has
        // since been re-run green.
        let facts = converted();
        let attempts: Vec<bool> = facts
            .iter()
            .flat_map(RepositoryFacts::pull_requests)
            .flat_map(|(facts, _)| facts.checks.iter())
            .map(|check| check.reported_at().is_some())
            .collect();
        assert!(
            attempts.len() > 1000,
            "there must be attempts to check, got {}",
            attempts.len()
        );
        assert_eq!(
            attempts.iter().filter(|dated| !**dated).count(),
            0,
            "every recorded attempt carries its instant"
        );
    }

    #[test]
    fn a_repeated_check_name_arrives_as_repeated_attempts() {
        // Reducing them is `classify`'s job, and it cannot do it if the
        // conversion collapsed them first.
        let facts = converted();
        let repeated = facts
            .iter()
            .flat_map(RepositoryFacts::pull_requests)
            .filter(|(facts, _)| {
                let mut names: Vec<&str> = facts.checks.iter().map(|c| c.name().as_str()).collect();
                let total = names.len();
                names.sort_unstable();
                names.dedup();
                names.len() < total
            })
            .count();
        assert_eq!(
            repeated, 10,
            "ten of the recorded pull requests repeat a check name"
        );
    }

    #[test]
    fn the_policy_a_pull_request_is_judged_by_follows_its_base_branch() {
        // The governed gap, on real data. The recording protects `main` with
        // four required checks; every other base branch is covered by no rule,
        // and judging one of those against `main`'s list would render its
        // failing checks as advisory when nothing says they are.
        let facts = converted();
        let judged: Vec<(String, bool, usize)> = facts
            .iter()
            .flat_map(RepositoryFacts::pull_requests)
            .map(|(pull_request, policy)| match &policy.blocking_checks {
                BlockingChecks::Only { required, .. } => (
                    pull_request.base_ref.as_str().to_owned(),
                    true,
                    required.as_slice().len(),
                ),
                BlockingChecks::Every => (pull_request.base_ref.as_str().to_owned(), false, 0),
            })
            .collect();
        assert_eq!(judged.len(), TOTAL_OPEN);

        let wrongly_governed: Vec<&(String, bool, usize)> = judged
            .iter()
            .filter(|(base, governed, _)| *governed != (base == "main"))
            .collect();
        assert!(
            wrongly_governed.is_empty(),
            "only a pull request into `main` is covered by the recorded rule: {wrongly_governed:?}"
        );
        assert!(
            judged.iter().any(|(_, governed, _)| *governed),
            "the recording holds pull requests into main"
        );
        assert!(
            judged.iter().any(|(_, governed, _)| !*governed),
            "and pull requests into branches no rule covers"
        );
        assert!(
            judged
                .iter()
                .filter(|(_, governed, _)| *governed)
                .all(|(_, _, required)| *required == 4),
            "the recorded rule names four required checks"
        );
    }

    #[test]
    fn a_status_context_becomes_a_check_beside_the_check_runs() {
        // The two kinds report the same thing in different words, and an
        // implementation reading only one loses every failure of the other.
        // Pinning the exact total is what makes that visible: 1489 check runs
        // and 40 status contexts, and nothing distinguishes them once
        // converted, so a dropped kind shows up only as a count that no longer
        // adds up.
        let facts = converted();
        let attempts: usize = facts
            .iter()
            .flat_map(RepositoryFacts::pull_requests)
            .map(|(facts, _)| facts.checks.len())
            .sum();
        assert_eq!(
            attempts, 1677,
            "every reported attempt of either kind must survive"
        );
    }

    #[test]
    fn a_status_context_is_read_from_its_own_fields() {
        // A status context names itself with `context` where a check run uses
        // `name`, reports through `state` rather than `conclusion`, and is
        // stamped `createdAt` rather than `startedAt`. Reading a check run's
        // field names off one would produce a nameless, stateless, undated
        // check.
        let only_a_status_context = converted_with(|pull_request| {
            for node in pull_request.commits.nodes.iter_mut().flatten().flatten() {
                if let Some(rollup) = node.commit.status_check_rollup.as_mut() {
                    rollup.contexts.nodes = Some(vec![Some(wire::CheckContext::StatusContext {
                        context: "ci/does-the-thing".to_owned(),
                        state: wire::StatusState::Failure,
                        created_at: "2026-08-24T11:24:06Z".to_owned(),
                    })]);
                }
            }
        });

        let first = only_a_status_context
            .as_ref()
            .and_then(|f| f.pull_requests().first())
            .and_then(|(facts, _)| facts.checks.first());
        assert_eq!(
            first.map(|check| check.name().as_str()),
            Some("ci/does-the-thing")
        );
        assert_eq!(
            first.map(|check| check.state().clone()),
            Some(CheckState::Failure)
        );
        assert_eq!(
            first.map(|check| check.reported_at().is_some()),
            Some(true),
            "a status context is stamped, and the instant orders it against re-runs"
        );
    }

    #[test]
    fn a_pull_request_with_no_commit_is_refused() {
        // Its checks and its push time are both statements about a commit that
        // is not there, so there is nothing honest to render.
        let refused = refusal(|pull_request| pull_request.commits.nodes = Some(Vec::new()));
        assert!(
            matches!(refused, Some(ConversionError::NoCommit { .. })),
            "a pull request without a commit is a fetch that went wrong: {refused:?}"
        );
    }

    #[test]
    fn an_unreadable_timestamp_is_refused_rather_than_dropped() {
        // Dropped to `None` it would lose the recency tie-break, and a
        // superseded failure would speak for a check since re-run green.
        let refused = refusal(|pull_request| {
            for node in pull_request.commits.nodes.iter_mut().flatten().flatten() {
                node.commit.committed_date = "the day before yesterday".to_owned();
            }
        });
        assert!(
            matches!(refused, Some(ConversionError::Timestamp { .. })),
            "expected the instant to be refused, got {refused:?}"
        );
    }

    #[test]
    fn a_number_no_pull_request_could_have_is_refused() {
        let refused = refusal(|pull_request| pull_request.number = -1);
        assert!(
            matches!(
                refused,
                Some(ConversionError::PullRequest { number: -1, .. })
            ),
            "expected the number to be refused, got {refused:?}"
        );
    }

    #[test]
    fn only_the_reviewers_who_asked_for_changes_are_named_as_blocking() {
        // Rung two of the ladder, and it outranks a failing required check. A
        // filter that let `COMMENTED` or `DISMISSED` through would report a
        // pull request as blocked on a human who said nothing of the kind.
        let facts = converted();
        let blocked_by: Vec<usize> = facts
            .iter()
            .flat_map(RepositoryFacts::pull_requests)
            .map(|(facts, _)| facts.changes_requested_by.len())
            .collect();

        assert_eq!(blocked_by.len(), TOTAL_OPEN);
        assert_eq!(
            blocked_by.iter().sum::<usize>(),
            3,
            "the recording holds three reviews asking for changes, against \
             every other review that says something else"
        );
        assert_eq!(
            blocked_by.iter().filter(|count| **count > 0).count(),
            3,
            "spread across three pull requests"
        );
    }

    #[test]
    fn everyone_still_being_waited_on_survives_the_conversion() {
        // The last rungs name who is being waited on, and fall through to
        // "nobody assigned" when this is empty. Losing a request renders a pull
        // request as waiting on nobody when somebody was asked.
        let facts = converted();
        let requested: Vec<usize> = facts
            .iter()
            .flat_map(RepositoryFacts::pull_requests)
            .map(|(facts, _)| facts.requested_reviewers.len())
            .collect();

        assert_eq!(requested.iter().sum::<usize>(), 85);
        assert_eq!(
            requested.iter().filter(|count| **count > 0).count(),
            29,
            "twenty-nine of the recorded pull requests are waiting on somebody"
        );
    }

    #[test]
    fn open_threads_and_comments_are_counted_as_reported() {
        // Threads block only where the policy demands resolution; comments
        // never block. Both are read straight, so the only way to get them
        // wrong is to count the wrong thing — resolved threads, say.
        let facts = converted();
        let counted: Vec<(u32, u32)> = facts
            .iter()
            .flat_map(RepositoryFacts::pull_requests)
            .map(|(facts, _)| (facts.unresolved_threads, facts.comments))
            .collect();

        assert_eq!(counted.len(), TOTAL_OPEN);
        assert_eq!(
            counted.iter().map(|(threads, _)| threads).sum::<u32>(),
            7,
            "the recording holds seven unresolved threads"
        );
        assert_eq!(
            counted.iter().map(|(_, comments)| *comments).max(),
            Some(19),
            "and one pull request with nineteen comments"
        );
    }

    #[test]
    fn githubs_own_unknown_and_a_value_nobody_can_read_stay_different_things() {
        // The two crates spell these the same way on purpose, and the mapping
        // is the one place they could be crossed. GitHub's `UNKNOWN` means it
        // has not finished computing; the catch-all means nobody could read the
        // value. Rendering the first as the second says "we do not understand
        // this" about a pull request GitHub is merely still thinking about; the
        // other way round hides a value no one has ever seen behind a spinner
        // that never stops. The recording holds no `UNKNOWN`, so nothing else
        // here would notice.
        let computing = converted_with(|pull_request| {
            pull_request.mergeable = wire::MergeableState::Computing;
            pull_request.merge_state_status = wire::MergeStateStatus::Undetermined;
        });
        assert_eq!(
            computing
                .as_ref()
                .and_then(|f| f.pull_requests().first())
                .map(|(facts, _)| (facts.mergeable.clone(), facts.merge_state.clone())),
            Some((MergeableState::Computing, MergeState::Undetermined)),
            "GitHub's own UNKNOWN is `Computing`/`Undetermined`, not the catch-all"
        );

        let unreadable = converted_with(|pull_request| {
            pull_request.mergeable = wire::MergeableState::Unknown("SOMETHING_NEW".to_owned());
            pull_request.merge_state_status =
                wire::MergeStateStatus::Unknown("SOMETHING_NEW".to_owned());
        });
        assert_eq!(
            unreadable
                .as_ref()
                .and_then(|f| f.pull_requests().first())
                .map(|(facts, _)| (facts.mergeable.clone(), facts.merge_state.clone())),
            Some((
                MergeableState::parse("SOMETHING_NEW"),
                MergeState::parse("SOMETHING_NEW")
            )),
            "and a value this crate cannot read keeps its text"
        );
    }

    #[test]
    fn a_draft_is_read_as_a_draft() {
        // The first rung, and it outranks every other reason.
        let facts = converted();
        let drafts = facts
            .iter()
            .flat_map(RepositoryFacts::pull_requests)
            .filter(|(facts, _)| facts.draft_state == DraftState::Draft)
            .count();
        assert_eq!(
            drafts, 15,
            "fifteen of the recorded pull requests are drafts"
        );
    }

    #[test]
    fn the_repository_facts_carry_the_branch_and_rule_a_policy_line_describes() {
        // The renderer's policy line and the JSON's `policy` object read these
        // two and nothing else. The data was fetched and parsed long before it
        // was reachable, which is the failure this pins: a field present in
        // `wire` but absent from the public surface is invisible to every test
        // that goes through the conversion.
        let facts = converted();

        assert_eq!(
            facts
                .as_ref()
                .and_then(RepositoryFacts::default_branch)
                .map(BranchName::as_str),
            Some("main"),
            "the recorded repository's default branch"
        );

        // Resolved through the same matcher every row uses. If these could
        // disagree, the policy line would describe one rule while the rows
        // beneath it were judged by another -- and the screen would look
        // entirely consistent while saying two different things.
        let disagreeing = facts
            .iter()
            .flat_map(RepositoryFacts::pull_requests)
            .filter(|(pull_request, _)| pull_request.base_ref.as_str() == "main")
            .filter(|(_, policy)| {
                facts
                    .as_ref()
                    .is_some_and(|facts| policy != facts.default_branch_rule())
            })
            .count();

        assert_eq!(
            disagreeing, 0,
            "every row on the default branch is judged by the rule the policy line names"
        );
    }

    #[test]
    fn the_facts_reduce_into_the_board_they_were_built_for() {
        // The contract this whole task feeds. If the shapes did not line up the
        // failure would be a compile error, but the counts are worth pinning:
        // every pull request must reach exactly one section.
        let parts = converted().map(RepositoryFacts::into_parts);
        assert!(parts.is_some(), "the recording converts");

        if let Some((viewer, _default_branch, _default_branch_rule, pull_requests)) = parts {
            let rows: Vec<BoardRow> = pull_requests
                .into_iter()
                .map(|(facts, policy)| BoardRow::reduce(facts, &policy))
                .collect();
            let board = Board::assemble(rows, &viewer);

            assert_eq!(board.total(), TOTAL_OPEN);
            assert_eq!(
                board.yours().len()
                    + board.needs_your_review().len()
                    + board.other_drafts().len()
                    + board.not_waiting_on_you().len(),
                TOTAL_OPEN,
                "every pull request lands in exactly one section"
            );
        }
    }
}
