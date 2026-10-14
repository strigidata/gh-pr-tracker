//! What changed since last time that is worth interrupting somebody for.
//!
//! The board is a debugging view of the state; this is the product. A nudge is
//! a *transition*, never a condition — "became ready to merge", not "is ready
//! to merge" — because a watcher that fires on a condition fires again every
//! few minutes until somebody acts, and that is the fatigue that gets a tool
//! uninstalled.
//!
//! # A false nudge costs more than a missed one
//!
//! Two consequences run through everything below. A pull request seen for the
//! first time nudges about nothing, because the first poll sees every open
//! pull request as new. And a reading GitHub has not finished computing is not
//! a transition: it is discarded rather than remembered, so a value that
//! flickers on every push does not announce itself each time it settles.

use crate::board::BoardRow;
use crate::decision::BlockedReason;
use crate::facts::ReviewDecision;
use crate::policy::{PolicyRules, StaleReviewHandling};
use crate::types::{HeadSha, Login};

/// A change worth interrupting somebody for.
///
/// Deliberately payload-free. A caller pairs one of these with the pull
/// request it came from, which it already has; putting the number and title in
/// here would duplicate the row and let the two drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nudge {
    /// Everything is out of the way. One click ends somebody else's wait, and
    /// this is the only nudge that asks for a pleasant thing.
    ReadyToMerge,
    /// A review that had already been given was thrown away by the base moving
    /// underneath it. Invisible without a watcher: nothing about the pull
    /// request changes on the day it happens except a field nobody reads.
    ApprovedThenConflicted,
    /// A check the repository requires went red.
    RequiredCheckFailed,
    /// The viewer has been asked to review something they had not been asked
    /// about before.
    ReviewRequestedOfYou,
    /// A push dismissed an approval, under a rule that dismisses them.
    ApprovalDismissedByPush,
    /// The author pushed after the viewer's standing review — approved or
    /// changes requested — so the next move is the viewer's again.
    ///
    /// GitHub keeps a changes-requested decision across a push, so without
    /// this the pull request goes on reading as waiting on its author.
    RereviewDue,
}

/// Which rung of the ladder a pull request was on, without the payload.
///
/// A discriminant-only mirror of [`BlockedReason`], because that is all a
/// transition needs and all that is worth carrying between polls. Keeping the
/// payload would make the remembered state grow with the number of reviewers
/// and would fire on changes — a second name joining a changes-requested list
/// — that are not the events this module reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasonKind {
    /// See [`BlockedReason::Draft`].
    Draft,
    /// See [`BlockedReason::ChangesRequested`].
    ChangesRequested,
    /// See [`BlockedReason::RequiredCheckFailing`].
    RequiredCheckFailing,
    /// See [`BlockedReason::UnresolvedThreads`].
    UnresolvedThreads,
    /// See [`BlockedReason::ApprovedButConflicting`].
    ApprovedButConflicting,
    /// See [`BlockedReason::Conflicting`].
    Conflicting,
    /// See [`BlockedReason::BehindBase`].
    BehindBase,
    /// See [`BlockedReason::ApprovedCiRunning`].
    ApprovedCiRunning,
    /// See [`BlockedReason::ReadyToMerge`].
    ReadyToMerge,
    /// See [`BlockedReason::CiRunning`].
    CiRunning,
    /// See [`BlockedReason::MergeabilityNotKnown`].
    ///
    /// The one rung that is not a settled answer. See [`Watched::reason`].
    MergeabilityNotKnown,
    /// See [`BlockedReason::WaitingForReview`].
    WaitingForReview,
    /// See [`BlockedReason::NobodyAssigned`].
    NobodyAssigned,
}

impl ReasonKind {
    /// Which rung this reason is.
    #[must_use]
    pub const fn of(reason: &BlockedReason) -> Self {
        match reason {
            BlockedReason::Draft => Self::Draft,
            BlockedReason::ChangesRequested { .. } => Self::ChangesRequested,
            BlockedReason::RequiredCheckFailing { .. } => Self::RequiredCheckFailing,
            BlockedReason::UnresolvedThreads { .. } => Self::UnresolvedThreads,
            BlockedReason::ApprovedButConflicting => Self::ApprovedButConflicting,
            BlockedReason::Conflicting => Self::Conflicting,
            BlockedReason::BehindBase => Self::BehindBase,
            BlockedReason::ApprovedCiRunning => Self::ApprovedCiRunning,
            BlockedReason::ReadyToMerge => Self::ReadyToMerge,
            BlockedReason::CiRunning => Self::CiRunning,
            BlockedReason::MergeabilityNotKnown => Self::MergeabilityNotKnown,
            BlockedReason::WaitingForReview { .. } => Self::WaitingForReview,
            BlockedReason::NobodyAssigned => Self::NobodyAssigned,
        }
    }

    /// Whether this is an answer or a shrug.
    ///
    /// [`ReasonKind::MergeabilityNotKnown`] is the only rung that means "ask
    /// again shortly": GitHub recomputes mergeability after every push and
    /// reports `UNKNOWN` for a few seconds while it does. Treating that as a
    /// state would make a pull request flicker out of and back into whatever
    /// it really is, announcing itself each time.
    const fn is_settled(self) -> bool {
        !matches!(self, Self::MergeabilityNotKnown)
    }
}

/// Whether the viewer has been asked to review.
///
/// A named pair rather than a `bool`, so a call site says which way round it
/// is. A team request does not count: [`crate::Reviewer::Team`] names no account, so
/// the viewer cannot be recognised in it even when they are a member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewRequest {
    /// The viewer is on the requested list by name.
    AskedOfYou,
    /// They are not.
    NotAskedOfYou,
}

/// Whether GitHub's review requirement is satisfied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalState {
    /// `reviewDecision` is `APPROVED`.
    Approved,
    /// It is anything else, including absent.
    NotApproved,
}

/// Whether anything the rule can block on is currently failing.
///
/// Named for health rather than for the checks themselves, because
/// [`crate::RequiredChecks`] is already the *set of names* a rule demands and
/// the two answer different questions.
///
/// Two states rather than a count: a second required check going red is the
/// same news as the first, and reporting it again is the fatigue this module
/// exists to avoid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequiredCheckHealth {
    /// At least one check the rule names is failing.
    SomeFailing,
    /// None is — which includes still running, and none reported at all.
    NoneFailing,
}

/// Whether the author has pushed since the viewer's standing review.
///
/// Two states rather than the [`crate::Verdict`] that
/// [`PrFacts::rereview_due`](crate::PrFacts::rereview_due) returns: the nudge
/// is the same news whichever way the viewer had answered, and carrying the
/// verdict would fire again when a reviewer flips from one to the other
/// without a push.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rereview {
    /// The viewer's review was made on a commit that is no longer the head.
    Due,
    /// It was not — no review, one on the head, or one GitHub gave no commit
    /// for.
    NotDue,
}

/// What the watcher carries about one pull request from one poll to the next.
///
/// Small on purpose: it is written to disk after every poll, and anything in
/// here is something a future version has to keep reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Watched {
    reason: ReasonKind,
    head: HeadSha,
    review_request: ReviewRequest,
    approval: ApprovalState,
    required_checks: RequiredCheckHealth,
    rereview: Rereview,
}

impl Watched {
    /// Assembles the state from a reading, without reference to any previous
    /// one.
    ///
    /// Private because a caller wants [`observe`], which also decides what to
    /// keep from before — this on its own would discard a settled reason in
    /// favour of a reading GitHub is still computing.
    fn of(row: &BoardRow, viewer: &Login) -> Self {
        let facts = row.facts();
        Self {
            reason: ReasonKind::of(row.reason()),
            head: facts.head.clone(),
            review_request: if facts
                .requested_reviewers
                .iter()
                .any(|reviewer| reviewer.login() == Some(viewer))
            {
                ReviewRequest::AskedOfYou
            } else {
                ReviewRequest::NotAskedOfYou
            },
            approval: match facts.review_decision {
                Some(ReviewDecision::Approved) => ApprovalState::Approved,
                _ => ApprovalState::NotApproved,
            },
            required_checks: if row.checks().blocking().is_empty() {
                RequiredCheckHealth::NoneFailing
            } else {
                RequiredCheckHealth::SomeFailing
            },
            rereview: if facts.rereview_due(viewer).is_some() {
                Rereview::Due
            } else {
                Rereview::NotDue
            },
        }
    }

    /// The last rung this pull request settled on.
    ///
    /// Never [`ReasonKind::MergeabilityNotKnown`] once anything settled has
    /// been seen: a reading GitHub had not finished computing is discarded in
    /// favour of the one before it. It can hold that value only when it is the
    /// only reading there has ever been, and a first sighting nudges about
    /// nothing anyway.
    #[must_use]
    pub const fn reason(&self) -> ReasonKind {
        self.reason
    }

    /// The commit this state was read at, so a push can be told from a change
    /// of mind.
    #[must_use]
    pub const fn head(&self) -> &HeadSha {
        &self.head
    }

    /// Whether the viewer was on the requested list.
    #[must_use]
    pub const fn review_request(&self) -> ReviewRequest {
        self.review_request
    }

    /// Whether the review requirement was satisfied.
    #[must_use]
    pub const fn approval(&self) -> ApprovalState {
        self.approval
    }

    /// Whether anything the rule blocks on was failing.
    #[must_use]
    pub const fn required_checks(&self) -> RequiredCheckHealth {
        self.required_checks
    }

    /// Whether the author had pushed since the viewer's standing review.
    #[must_use]
    pub const fn rereview(&self) -> Rereview {
        self.rereview
    }

    /// Rebuilds a remembered state, for a caller reading one back off disk.
    ///
    /// The watcher's state file is this crate's shape written by the shell, so
    /// there has to be a way back in. Nothing is validated beyond the types,
    /// because every field is already one that cannot hold a wrong value.
    #[must_use]
    pub const fn restored(
        reason: ReasonKind,
        head: HeadSha,
        review_request: ReviewRequest,
        approval: ApprovalState,
        required_checks: RequiredCheckHealth,
        rereview: Rereview,
    ) -> Self {
        Self {
            reason,
            head,
            review_request,
            approval,
            required_checks,
            rereview,
        }
    }
}

/// What one poll of one pull request produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    /// What to carry into the next poll.
    pub remembered: Watched,
    /// What changed, if anything. Usually empty — most polls see nothing.
    pub nudges: Vec<Nudge>,
}

/// Compares this reading against the last one and says what is worth saying.
///
/// `before` is `None` for a pull request this watcher has not seen, which
/// nudges about nothing: the first poll sees every open pull request as new,
/// and announcing all of them is how a watcher gets closed before its second
/// poll.
///
/// `policy` is the rule governing *this* pull request, not the repository's
/// default — whether a push dismisses approvals is a property of the rule that
/// covers the branch, and a stacked pull request is covered by none.
#[must_use]
pub fn observe(
    before: Option<&Watched>,
    row: &BoardRow,
    viewer: &Login,
    policy: &PolicyRules,
) -> Observation {
    let now = Watched::of(row, viewer);

    let Some(before) = before else {
        return Observation {
            remembered: now,
            nudges: Vec::new(),
        };
    };

    // A reading GitHub has not finished computing is not an event and does not
    // replace what we knew. Everything below compares against the last settled
    // answer, so a value that flickers on every push is invisible here.
    let settled = now.reason.is_settled();
    let remembered = Watched {
        reason: if settled { now.reason } else { before.reason },
        ..now.clone()
    };

    // Neither rung below needs to ask whether the reading settled: both name
    // a specific rung, and a reading GitHub is still computing is on none of
    // them. The protection against a flapping value comes entirely from
    // comparing against `before.reason`, which the carry-forward above keeps
    // at the last settled answer. Guarding these as well reads like a second
    // defence and is not one — mutation testing showed the guard could be
    // deleted with no test noticing, because it can never be false here.
    let mut nudges = Vec::new();
    if before.reason != ReasonKind::ReadyToMerge && now.reason == ReasonKind::ReadyToMerge {
        nudges.push(Nudge::ReadyToMerge);
    }
    if before.reason != ReasonKind::ApprovedButConflicting
        && now.reason == ReasonKind::ApprovedButConflicting
    {
        nudges.push(Nudge::ApprovedThenConflicted);
    }
    // Read off the checks rather than off the ladder, because a pull request
    // already blocked on something that outranks a failure would otherwise
    // hide it — and the author has to fix the check either way.
    if before.required_checks == RequiredCheckHealth::NoneFailing
        && now.required_checks == RequiredCheckHealth::SomeFailing
    {
        nudges.push(Nudge::RequiredCheckFailed);
    }
    if before.review_request == ReviewRequest::NotAskedOfYou
        && now.review_request == ReviewRequest::AskedOfYou
    {
        nudges.push(Nudge::ReviewRequestedOfYou);
    }
    if before.rereview == Rereview::NotDue && now.rereview == Rereview::Due {
        nudges.push(Nudge::RereviewDue);
    }
    if dismissed_by_push(before, &now, policy) {
        nudges.push(Nudge::ApprovalDismissedByPush);
    }

    Observation { remembered, nudges }
}

/// Whether an approval was lost *to a push*, rather than to a reviewer
/// changing their mind.
///
/// Three things have to hold at once, and the rule is the one most easily
/// skipped: on a repository that keeps approvals across a push, the same
/// readings mean nothing happened. Naming the wrong cause in a notification is
/// worse than staying quiet, because the reader acts on the cause.
fn dismissed_by_push(before: &Watched, now: &Watched, policy: &PolicyRules) -> bool {
    matches!(policy.stale_reviews, StaleReviewHandling::DismissOnPush)
        && before.approval == ApprovalState::Approved
        && now.approval == ApprovalState::NotApproved
        && before.head != now.head
}

#[cfg(test)]
mod tests {
    use super::{observe, Nudge, ReasonKind, Watched};
    use crate::board::BoardRow;
    use crate::checks::{Check, CheckState};
    use crate::facts::{
        BranchName, DraftState, HeadRepository, MergeState, MergeableState, PrFacts, PrTitle,
        PrUrl, Review, ReviewDecision, Reviewer, TeamSlug, Verdict,
    };
    use crate::policy::{BlockingChecks, PolicyRules, Requirement, StaleReviewHandling};
    use crate::types::{CheckName, HeadSha, InvalidValue, Login, PrNumber, RepoRef};
    use chrono::{DateTime, TimeDelta, Utc};

    /// Two object ids, so a push can be expressed as the head moving.
    const FIRST_HEAD: &str = "9f2c4e1b7a3d5068c9e4b2a17f36d8095c1e4a7b";
    const AFTER_A_PUSH: &str = "1a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d";

    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::<Utc>::UNIX_EPOCH + TimeDelta::minutes(minutes)
    }

    fn open_pr() -> Result<PrFacts, InvalidValue> {
        Ok(PrFacts {
            repo: RepoRef::parse("octo-org/example")?,
            number: PrNumber::new(1)?,
            url: PrUrl::new("https://github.com/octo-org/example/pull/1"),
            title: PrTitle::new("Split the ledger writer"),
            author: Some(Login::parse("octocat")?),
            draft_state: DraftState::ReadyForReview,
            head: HeadSha::parse(FIRST_HEAD)?,
            head_ref: BranchName::new("topic/branch-01"),
            head_repository: HeadRepository::ThisOne,
            base_ref: BranchName::new("main"),
            review_decision: None,
            mergeable: MergeableState::Mergeable,
            merge_state: MergeState::Clean,
            requested_reviewers: Vec::new(),
            changes_requested_by: Vec::new(),
            reviews: Vec::new(),
            unresolved_threads: 0,
            comments: 0,
            checks: Vec::new(),
            last_push: at(0),
        })
    }

    /// A rule that names a required check and dismisses approvals on a push.
    fn strict_rule() -> Result<PolicyRules, InvalidValue> {
        Ok(PolicyRules {
            blocking_checks: BlockingChecks::from_rule(
                Requirement::Required,
                vec![CheckName::parse("Preview CI Gate")?],
            ),
            required_approvals: 1,
            conversation_resolution: Requirement::NotRequired,
            code_owner_reviews: Requirement::NotRequired,
            up_to_date_branch: Requirement::NotRequired,
            stale_reviews: StaleReviewHandling::DismissOnPush,
        })
    }

    fn check(state: &str) -> Result<Check, InvalidValue> {
        Ok(Check::new(
            CheckName::parse("Preview CI Gate")?,
            CheckState::parse(state),
            Some(at(0)),
        ))
    }

    /// Feeds a sequence of readings through the watcher, returning what each
    /// one nudged about.
    ///
    /// The state is carried from one to the next exactly as the watch loop
    /// carries it, so a test can assert on the *sequence* — which is the only
    /// way to catch a transition firing twice.
    fn watch(readings: Vec<PrFacts>, policy: &PolicyRules, viewer: &Login) -> Vec<Vec<Nudge>> {
        let mut remembered: Option<Watched> = None;
        let mut fired = Vec::new();
        for facts in readings {
            let row = BoardRow::reduce(facts, policy);
            let seen = observe(remembered.as_ref(), &row, viewer, policy);
            fired.push(seen.nudges.clone());
            remembered = Some(seen.remembered);
        }
        fired
    }

    #[test]
    fn a_pull_request_seen_for_the_first_time_never_nudges() {
        // The most expensive mistake this module could make. The first poll
        // sees every open pull request as new, and a tool that announces all
        // of them on startup gets closed before its second poll — the plan
        // records nineteen misdirected messages from exactly this shape.
        let fired = Login::parse("octocat").and_then(|me| {
            let mut ready = open_pr()?;
            ready.review_decision = Some(ReviewDecision::Approved);
            ready.checks = vec![check("SUCCESS")?];
            Ok::<_, InvalidValue>(watch(vec![ready], &strict_rule()?, &me))
        });

        assert_eq!(
            fired,
            Ok(vec![Vec::new()]),
            "a first sighting is a baseline, not news"
        );
    }

    #[test]
    fn becoming_ready_to_merge_nudges_once_and_then_stays_quiet() {
        // The nudge the whole tool exists for, and the one most at risk of
        // repeating: nothing about the pull request changes on the polls after
        // it, so a watcher comparing against "is it ready" rather than "did it
        // become ready" would fire every few minutes forever.
        let fired = Login::parse("octocat").and_then(|me| {
            let mut waiting = open_pr()?;
            waiting.checks = vec![check("IN_PROGRESS")?];

            let mut ready = open_pr()?;
            ready.review_decision = Some(ReviewDecision::Approved);
            ready.checks = vec![check("SUCCESS")?];

            Ok::<_, InvalidValue>(watch(
                vec![waiting, ready.clone(), ready],
                &strict_rule()?,
                &me,
            ))
        });

        assert_eq!(
            fired,
            Ok(vec![Vec::new(), vec![Nudge::ReadyToMerge], Vec::new()])
        );
    }

    #[test]
    fn approved_work_that_rots_into_a_conflict_nudges() {
        // The case the board was built to surface and the one the user hit
        // personally: a review already given, thrown away because the base
        // moved. Invisible without a watcher, because nothing about the pull
        // request changes on the day it happens except a field nobody reads.
        let fired = Login::parse("octocat").and_then(|me| {
            let mut approved = open_pr()?;
            approved.review_decision = Some(ReviewDecision::Approved);
            approved.checks = vec![check("SUCCESS")?];

            let mut rotted = approved.clone();
            rotted.mergeable = MergeableState::Conflicting;

            // Three readings, not two: with only "before" and "after", firing
            // on the condition and firing on the transition are the same
            // answer. The third says the conflict is still there and must be
            // silent, which is what tells them apart -- a pull request that
            // has rotted stays rotted for days.
            Ok::<_, InvalidValue>(watch(
                vec![approved, rotted.clone(), rotted],
                &strict_rule()?,
                &me,
            ))
        });

        assert_eq!(
            fired,
            Ok(vec![
                Vec::new(),
                vec![Nudge::ApprovedThenConflicted],
                Vec::new()
            ])
        );
    }

    #[test]
    fn a_required_check_going_red_nudges_even_when_something_outranks_it() {
        // Read off the checks rather than off the ladder's top rung. A pull
        // request already blocked on a review would hide the failure entirely
        // if this asked the ladder, and "your required check broke" is
        // actionable on its own -- the author has to fix it either way.
        let fired = Login::parse("octocat").and_then(|me| {
            let mut green = open_pr()?;
            green.changes_requested_by = vec![Login::parse("hubot")?];
            green.checks = vec![check("SUCCESS")?];

            let mut red = green.clone();
            red.checks = vec![check("FAILURE")?];

            // A third reading, still red, for the reason the conflict test
            // needs one: two readings cannot tell a transition from a
            // condition, and a red check stays red until somebody fixes it.
            Ok::<_, InvalidValue>(watch(vec![green, red.clone(), red], &strict_rule()?, &me))
        });

        assert_eq!(
            fired,
            Ok(vec![
                Vec::new(),
                vec![Nudge::RequiredCheckFailed],
                Vec::new()
            ]),
            "changes-requested outranks it on the board, but the failure is still news"
        );
    }

    #[test]
    fn being_asked_for_a_review_nudges_once() {
        let fired = Login::parse("octocat").and_then(|me| {
            let mut unasked = open_pr()?;
            unasked.author = Some(Login::parse("hubot")?);

            let mut asked = unasked.clone();
            asked.requested_reviewers = vec![Reviewer::User(me.clone())];

            Ok::<_, InvalidValue>(watch(
                vec![unasked, asked.clone(), asked],
                &strict_rule()?,
                &me,
            ))
        });

        assert_eq!(
            fired,
            Ok(vec![
                Vec::new(),
                vec![Nudge::ReviewRequestedOfYou],
                Vec::new()
            ])
        );
    }

    #[test]
    fn a_team_whose_slug_reads_like_your_login_is_not_a_request_to_you() {
        // A team names no account, so the viewer cannot be recognised in one
        // even when they are a member. The slug here is deliberately the
        // viewer's own handle: a probe that could not collide passes just as
        // happily against a check that compares rendered names, and would let
        // a team request nudge somebody who was never asked.
        let fired = Login::parse("octocat").and_then(|me| {
            let mut theirs = open_pr()?;
            theirs.author = Some(Login::parse("hubot")?);

            let mut asked_the_team = theirs.clone();
            asked_the_team.requested_reviewers = vec![Reviewer::Team(TeamSlug::new("octocat"))];

            Ok::<_, InvalidValue>(watch(vec![theirs, asked_the_team], &strict_rule()?, &me))
        });

        assert_eq!(fired, Ok(vec![Vec::new(), Vec::new()]));
    }

    #[test]
    fn a_review_requested_of_somebody_else_is_not_your_business() {
        let fired = Login::parse("octocat").and_then(|me| {
            let mut theirs = open_pr()?;
            theirs.author = Some(Login::parse("hubot")?);

            let mut asked = theirs.clone();
            asked.requested_reviewers = vec![Reviewer::User(Login::parse("someone-else")?)];

            Ok::<_, InvalidValue>(watch(vec![theirs, asked], &strict_rule()?, &me))
        });

        assert_eq!(fired, Ok(vec![Vec::new(), Vec::new()]));
    }

    #[test]
    fn a_push_that_dismisses_an_approval_nudges_only_where_the_rule_dismisses() {
        // Three things have to hold together, and the rule is the one a
        // watcher is most likely to skip: on a repository that keeps approvals
        // across a push, the same sequence of readings means nothing happened.
        let dismissed = Login::parse("octocat").and_then(|me| {
            let mut approved = open_pr()?;
            approved.review_decision = Some(ReviewDecision::Approved);
            approved.checks = vec![check("SUCCESS")?];

            let mut pushed = approved.clone();
            pushed.head = HeadSha::parse(AFTER_A_PUSH)?;
            pushed.review_decision = Some(ReviewDecision::ReviewRequired);

            let strict = strict_rule()?;
            let lenient = PolicyRules {
                stale_reviews: StaleReviewHandling::Keep,
                ..strict.clone()
            };
            Ok::<_, InvalidValue>((
                watch(vec![approved.clone(), pushed.clone()], &strict, &me),
                watch(vec![approved, pushed], &lenient, &me),
            ))
        });

        assert_eq!(
            dismissed,
            Ok((
                vec![Vec::new(), vec![Nudge::ApprovalDismissedByPush]],
                vec![Vec::new(), Vec::new()],
            )),
            "the same readings are news under one rule and silence under the other"
        );
    }

    #[test]
    fn a_push_that_leaves_the_approval_standing_is_not_a_dismissal() {
        // The rule dismisses on push and the head moved, but the approval is
        // still there -- somebody re-approved between polls, or GitHub had not
        // applied the dismissal when we looked. Announcing a dismissal that
        // did not happen sends the author to chase a reviewer who has already
        // signed off, which is the false nudge in its purest form.
        let fired = Login::parse("octocat").and_then(|me| {
            let mut approved = open_pr()?;
            approved.review_decision = Some(ReviewDecision::Approved);
            approved.checks = vec![check("SUCCESS")?];

            let mut pushed_and_reapproved = approved.clone();
            pushed_and_reapproved.head = HeadSha::parse(AFTER_A_PUSH)?;

            Ok::<_, InvalidValue>(watch(
                vec![approved, pushed_and_reapproved],
                &strict_rule()?,
                &me,
            ))
        });

        assert_eq!(fired, Ok(vec![Vec::new(), Vec::new()]));
    }

    #[test]
    fn an_approval_that_disappears_without_a_push_is_not_a_dismissal() {
        // A reviewer changing their mind is not the same event, and calling it
        // one would name the wrong cause in the notification.
        let fired = Login::parse("octocat").and_then(|me| {
            let mut approved = open_pr()?;
            approved.review_decision = Some(ReviewDecision::Approved);
            approved.checks = vec![check("SUCCESS")?];

            let mut withdrawn = approved.clone();
            withdrawn.review_decision = Some(ReviewDecision::ReviewRequired);

            Ok::<_, InvalidValue>(watch(vec![approved, withdrawn], &strict_rule()?, &me))
        });

        assert_eq!(fired, Ok(vec![Vec::new(), Vec::new()]));
    }

    /// Somebody else's pull request on which the viewer asked for changes at
    /// [`FIRST_HEAD`], read with its head at `head`.
    fn reviewed_by_you_at_first_head(me: &Login, head: &str) -> Result<PrFacts, InvalidValue> {
        let mut theirs = open_pr()?;
        theirs.author = Some(Login::parse("hubot")?);
        theirs.head = HeadSha::parse(head)?;
        theirs.changes_requested_by = vec![me.clone()];
        theirs.reviews = vec![Review {
            by: me.clone(),
            verdict: Verdict::ChangesRequested,
            on: Some(HeadSha::parse(FIRST_HEAD)?),
        }];
        Ok(theirs)
    }

    #[test]
    fn a_push_after_your_review_nudges_once_and_then_stays_quiet() {
        // Three readings, for the reason the conflict test gives: a before and
        // an after cannot tell "became due" from "is due". The third reading
        // is still due -- nobody re-reviews within one poll -- and must be
        // silent, or this fires every few minutes until somebody looks.
        let fired = Login::parse("octocat").and_then(|me| {
            Ok::<_, InvalidValue>(watch(
                vec![
                    reviewed_by_you_at_first_head(&me, FIRST_HEAD)?,
                    reviewed_by_you_at_first_head(&me, AFTER_A_PUSH)?,
                    reviewed_by_you_at_first_head(&me, AFTER_A_PUSH)?,
                ],
                &strict_rule()?,
                &me,
            ))
        });

        assert_eq!(
            fired,
            Ok(vec![Vec::new(), vec![Nudge::RereviewDue], Vec::new()])
        );
    }

    #[test]
    fn a_re_review_already_due_on_first_sighting_does_not_nudge() {
        // The same rule as every other first sighting: the watcher's first
        // poll sees every stale review at once, and announcing all of them is
        // a startup burst, not news.
        let fired = Login::parse("octocat").and_then(|me| {
            Ok::<_, InvalidValue>(watch(
                vec![reviewed_by_you_at_first_head(&me, AFTER_A_PUSH)?],
                &strict_rule()?,
                &me,
            ))
        });

        assert_eq!(fired, Ok(vec![Vec::new()]));
    }

    #[test]
    fn a_mergeability_github_is_still_computing_never_nudges_and_never_forgets() {
        // The transient that would make this unusable. Every push sets
        // `mergeable` to UNKNOWN for a few seconds, so a pull request that was
        // ready to merge flickers ready -> computing -> ready. Remembering the
        // middle reading would announce "ready to merge" a second time on every
        // push, which is the fatigue that gets the tool uninstalled.
        //
        // So the computing reading nudges nothing and is not remembered: the
        // watcher keeps the last settled answer and sees no change when it
        // returns.
        let fired = Login::parse("octocat").and_then(|me| {
            let mut ready = open_pr()?;
            ready.review_decision = Some(ReviewDecision::Approved);
            ready.checks = vec![check("SUCCESS")?];

            let mut computing = ready.clone();
            computing.mergeable = MergeableState::Computing;

            let mut waiting = open_pr()?;
            waiting.checks = vec![check("IN_PROGRESS")?];

            Ok::<_, InvalidValue>(watch(
                vec![waiting, ready.clone(), computing, ready],
                &strict_rule()?,
                &me,
            ))
        });

        assert_eq!(
            fired,
            Ok(vec![
                Vec::new(),
                vec![Nudge::ReadyToMerge],
                Vec::new(),
                Vec::new(),
            ]),
            "ready, then computing, then ready again is one event and not three"
        );
    }

    #[test]
    fn the_settled_reason_is_what_is_remembered_across_a_computing_reading() {
        // The mechanism behind the test above, asserted directly so a future
        // change to the carry-forward fails here with a legible reason rather
        // than as a duplicate notification three tests away.
        let remembered = Login::parse("octocat").and_then(|me| {
            let mut ready = open_pr()?;
            ready.review_decision = Some(ReviewDecision::Approved);
            ready.checks = vec![check("SUCCESS")?];
            let mut computing = ready.clone();
            computing.mergeable = MergeableState::Computing;

            let policy = strict_rule()?;
            let settled = observe(None, &BoardRow::reduce(ready, &policy), &me, &policy);
            let during = observe(
                Some(&settled.remembered),
                &BoardRow::reduce(computing, &policy),
                &me,
                &policy,
            );
            Ok::<_, InvalidValue>((settled.remembered.reason(), during.remembered.reason()))
        });

        assert_eq!(
            remembered,
            Ok((ReasonKind::ReadyToMerge, ReasonKind::ReadyToMerge)),
            "the computing reading must not overwrite the settled one"
        );
    }
}
