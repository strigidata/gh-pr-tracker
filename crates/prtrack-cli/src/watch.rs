//! One poll's worth of watching, with no clock and no socket in sight.
//!
//! Everything here is a pure function over readings the caller already has, so
//! the loop around it has nothing left to get wrong but sleeping and fetching.
//! That split is deliberate: a test for the interval can then use a virtual
//! clock, which only works while nothing touches a socket.

use prtrack_core::{
    observe, BlockedReason, BoardRow, DraftState, Login, Nudge, PolicyRules, PrNumber, PrTitle,
    PrUrl,
};

use crate::state::WatchState;

/// A nudge, and enough of the pull request to say it out loud.
///
/// A notification reading "a pull request is ready" is one the reader has to
/// go and decode, which is most of the way back to not being told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nudged {
    /// Which pull request.
    pub number: PrNumber,
    /// What it is called, for the body of the notification.
    pub title: PrTitle,
    /// Where to go, for the click.
    pub url: PrUrl,
    /// What changed.
    pub nudge: Nudge,
}

impl Nudged {
    /// What changed, as the phrase that follows the pull request number.
    ///
    /// Separate from [`summary`] so a caller can put the number and the phrase
    /// in different places — a terminal makes the number a link, a notification
    /// cannot.
    ///
    /// [`summary`]: Nudged::summary
    #[must_use]
    pub const fn happened(&self) -> &'static str {
        match self.nudge {
            Nudge::ReadyToMerge => "is ready to merge",
            Nudge::ApprovedThenConflicted => "was approved and now conflicts",
            Nudge::RequiredCheckFailed => "has a required check failing",
            Nudge::ReviewRequestedOfYou => "is waiting for your review",
            Nudge::ApprovalDismissedByPush => "lost its approval to a push",
        }
    }

    /// The number as it is written, with its `#`.
    #[must_use]
    pub fn label(&self) -> String {
        format!("#{}", self.number.get())
    }

    /// The one-line form, in plain text.
    ///
    /// This is what a desktop notification gets, so it must stay free of escape
    /// sequences — a notification body is not a terminal and would render them
    /// literally.
    #[must_use]
    pub fn summary(&self) -> String {
        format!("{} {}", self.label(), self.happened())
    }
}

/// Folds one poll's readings into the remembered state, and says what changed.
///
/// `judged` is every pull request the poll saw, each beside the rule that
/// judged it. **Only call this with a poll that succeeded.** A fetch that was
/// refused is not an observation: diffing against a partial or absent reading
/// invents transitions that did not happen and, worse, hides ones that did,
/// which is a nudge that never fires and therefore never gets noticed.
///
/// Pull requests that did not appear are forgotten, so the state file cannot
/// grow for the life of the repository and a reopened number is not compared
/// against a reason recorded before it closed.
pub fn tick(
    state: &mut WatchState,
    judged: &[(BoardRow, PolicyRules)],
    viewer: &Login,
) -> Vec<Nudged> {
    let mut nudged = Vec::new();

    for (row, policy) in judged {
        let facts = row.facts();
        let seen = observe(state.remembered(facts.number), row, viewer, policy);
        for nudge in seen.nudges {
            nudged.push(Nudged {
                number: facts.number,
                title: facts.title.clone(),
                url: facts.url.clone(),
                nudge,
            });
        }
        state.remember(facts.number, seen.remembered);
    }

    let still_open: Vec<PrNumber> = judged.iter().map(|(row, _)| row.facts().number).collect();
    state.retain_only(&still_open);

    nudged
}

/// What is already worth doing on a board nobody has been notified about.
///
/// Every one of these is a *condition*, so none will ever produce a nudge —
/// the watcher reports transitions. That is exactly why the first run has to
/// count them: a pull request that has been approved-and-conflicted for three
/// weeks is the situation this tool exists to surface, it is on the screen,
/// and it is never going to announce itself.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AlreadyWaiting {
    /// Yours, with nothing left in the way.
    pub ready_to_merge: usize,
    /// Yours, approved and then overtaken by the base.
    pub approved_but_conflicting: usize,
    /// Yours, with a check the rule demands going red.
    pub required_check_failing: usize,
    /// Somebody else's, with your review outstanding.
    pub awaiting_your_review: usize,
}

impl AlreadyWaiting {
    /// The clause for the startup line, or `None` when there is nothing to
    /// say.
    ///
    /// `None` rather than a row of zeroes, so the caller can leave the whole
    /// clause out. "0 ready to merge, 0 approved but conflicting" is noise
    /// that trains a reader to skip the line the one time it matters.
    #[must_use]
    pub fn describe(&self) -> Option<String> {
        let counted = [
            (self.ready_to_merge, "ready to merge"),
            (self.approved_but_conflicting, "approved but conflicting"),
            (self.required_check_failing, "with a required check failing"),
            (self.awaiting_your_review, "waiting on your review"),
        ];
        let said: Vec<String> = counted
            .iter()
            .filter(|(count, _)| *count > 0)
            .map(|(count, what)| format!("{count} {what}"))
            .collect();
        (!said.is_empty()).then(|| said.join(", "))
    }
}

/// Counts what the board already shows that the viewer could act on.
///
/// Restricted to the viewer's own work for the first three, because the count
/// answers "is this thing doing anything for me". Somebody else's failing
/// check is their problem, and counting it would inflate the one number the
/// startup line exists to make trustworthy.
#[must_use]
pub fn already_waiting(judged: &[(BoardRow, PolicyRules)], viewer: &Login) -> AlreadyWaiting {
    let mut waiting = AlreadyWaiting::default();

    for (row, _) in judged {
        let facts = row.facts();
        if facts.draft_state == DraftState::Draft {
            continue;
        }
        let yours = facts.author.as_ref() == Some(viewer);

        if yours {
            match row.reason() {
                BlockedReason::ReadyToMerge => waiting.ready_to_merge += 1,
                BlockedReason::ApprovedButConflicting => waiting.approved_but_conflicting += 1,
                BlockedReason::RequiredCheckFailing { .. } => {
                    waiting.required_check_failing += 1;
                }
                BlockedReason::Draft
                | BlockedReason::ChangesRequested { .. }
                | BlockedReason::UnresolvedThreads { .. }
                | BlockedReason::Conflicting
                | BlockedReason::BehindBase
                | BlockedReason::ApprovedCiRunning
                | BlockedReason::CiRunning
                | BlockedReason::MergeabilityNotKnown
                | BlockedReason::WaitingForReview { .. }
                | BlockedReason::NobodyAssigned => {}
            }
        } else if facts
            .requested_reviewers
            .iter()
            .any(|reviewer| reviewer.login() == Some(viewer))
        {
            waiting.awaiting_your_review += 1;
        }
    }

    waiting
}

#[cfg(test)]
mod tests {
    use super::{already_waiting, tick, Nudged};
    use crate::state::WatchState;
    use chrono::{DateTime, TimeDelta, Utc};
    use prtrack_core::{
        BlockingChecks, BoardRow, BranchName, Check, CheckName, CheckState, DraftState, HeadSha,
        InvalidValue, Login, MergeState, MergeableState, Nudge, PolicyRules, PrFacts, PrNumber,
        PrTitle, PrUrl, RepoRef, Requirement, ReviewDecision, Reviewer, StaleReviewHandling,
    };

    const A_HEAD: &str = "9f2c4e1b7a3d5068c9e4b2a17f36d8095c1e4a7b";

    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::<Utc>::UNIX_EPOCH + TimeDelta::minutes(minutes)
    }

    fn open_pr(number: u32, author: &str) -> Result<PrFacts, InvalidValue> {
        Ok(PrFacts {
            repo: RepoRef::parse("octo-org/example")?,
            number: PrNumber::new(number)?,
            url: PrUrl::new("https://github.com/octo-org/example/pull/1"),
            title: PrTitle::new("Split the ledger writer"),
            author: Some(Login::parse(author)?),
            draft_state: DraftState::ReadyForReview,
            head: HeadSha::parse(A_HEAD)?,
            head_ref: BranchName::new("topic/branch-01"),
            base_ref: BranchName::new("main"),
            review_decision: None,
            mergeable: MergeableState::Mergeable,
            merge_state: MergeState::Clean,
            requested_reviewers: Vec::new(),
            changes_requested_by: Vec::new(),
            unresolved_threads: 0,
            comments: 0,
            checks: Vec::new(),
            last_push: at(0),
        })
    }

    fn rule() -> Result<PolicyRules, InvalidValue> {
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

    /// Reduces readings into rows paired with the rule that judged them.
    fn judged(prs: Vec<PrFacts>, policy: &PolicyRules) -> Vec<(BoardRow, PolicyRules)> {
        prs.into_iter()
            .map(|facts| (BoardRow::reduce(facts, policy), policy.clone()))
            .collect()
    }

    #[test]
    fn the_first_poll_records_everything_and_announces_nothing() {
        let outcome = Login::parse("octocat").and_then(|me| {
            let mut ready = open_pr(1, "octocat")?;
            ready.review_decision = Some(ReviewDecision::Approved);
            ready.checks = vec![check("SUCCESS")?];

            let policy = rule()?;
            let mut state = WatchState::default();
            let nudged = tick(
                &mut state,
                &judged(vec![ready, open_pr(2, "hubot")?], &policy),
                &me,
            );
            Ok::<_, InvalidValue>((nudged.len(), state.len()))
        });

        assert_eq!(
            outcome,
            Ok((0, 2)),
            "both are remembered, and neither is news"
        );
    }

    #[test]
    fn a_second_poll_reports_only_what_moved() {
        let outcome = Login::parse("octocat").and_then(|me| {
            let policy = rule()?;
            let mut waiting = open_pr(1, "octocat")?;
            waiting.checks = vec![check("IN_PROGRESS")?];
            let quiet = open_pr(2, "hubot")?;

            let mut state = WatchState::default();
            let _baseline = tick(
                &mut state,
                &judged(vec![waiting, quiet.clone()], &policy),
                &me,
            );

            let mut ready = open_pr(1, "octocat")?;
            ready.review_decision = Some(ReviewDecision::Approved);
            ready.checks = vec![check("SUCCESS")?];

            let nudged = tick(&mut state, &judged(vec![ready, quiet], &policy), &me);
            Ok::<_, InvalidValue>(
                nudged
                    .iter()
                    .map(|n| (n.number.get(), n.nudge))
                    .collect::<Vec<_>>(),
            )
        });

        assert_eq!(outcome, Ok(vec![(1, Nudge::ReadyToMerge)]));
    }

    #[test]
    fn a_pull_request_that_closed_stops_being_remembered() {
        // The prune runs off the poll that just happened, so a pull request
        // that stopped appearing is forgotten rather than kept forever.
        let outcome = Login::parse("octocat").and_then(|me| {
            let policy = rule()?;
            let mut state = WatchState::default();
            let _first = tick(
                &mut state,
                &judged(
                    vec![open_pr(1, "octocat")?, open_pr(2, "octocat")?],
                    &policy,
                ),
                &me,
            );
            let _second = tick(
                &mut state,
                &judged(vec![open_pr(1, "octocat")?], &policy),
                &me,
            );
            Ok::<_, InvalidValue>((state.len(), state.remembered(PrNumber::new(2)?).is_some()))
        });

        assert_eq!(outcome, Ok((1, false)));
    }

    #[test]
    fn a_nudge_carries_what_a_notification_has_to_say() {
        // The number, the title and the link. A notification that says only
        // "a pull request is ready" is one the reader has to go and decode,
        // which is most of the way back to not being told at all.
        let outcome = Login::parse("octocat").and_then(|me| {
            let policy = rule()?;
            let mut before = open_pr(4225, "octocat")?;
            before.title = PrTitle::new("Fix the flaky gate");
            before.url = PrUrl::new("https://github.com/octo-org/example/pull/4225");
            before.checks = vec![check("IN_PROGRESS")?];

            let mut after = before.clone();
            after.review_decision = Some(ReviewDecision::Approved);
            after.checks = vec![check("SUCCESS")?];

            let mut state = WatchState::default();
            let _baseline = tick(&mut state, &judged(vec![before], &policy), &me);
            let nudged = tick(&mut state, &judged(vec![after], &policy), &me);

            Ok::<_, InvalidValue>(nudged.first().map(|n| {
                (
                    n.number.get(),
                    n.title.as_str().to_owned(),
                    n.url.as_str().to_owned(),
                )
            }))
        });

        assert_eq!(
            outcome,
            Ok(Some((
                4225,
                "Fix the flaky gate".to_owned(),
                "https://github.com/octo-org/example/pull/4225".to_owned()
            )))
        );
    }

    #[test]
    fn the_first_run_counts_what_is_already_worth_doing() {
        // The startup line's whole job. These are conditions rather than
        // transitions, so none of them will ever produce a nudge -- a first
        // run that said only "nothing until something changes" would imply
        // there is nothing to do, which is the opposite of the point.
        let counted = Login::parse("octocat").and_then(|me| {
            let policy = rule()?;

            let mut mergeable = open_pr(1, "octocat")?;
            mergeable.review_decision = Some(ReviewDecision::Approved);
            mergeable.checks = vec![check("SUCCESS")?];

            let mut rotted = open_pr(2, "octocat")?;
            rotted.review_decision = Some(ReviewDecision::Approved);
            rotted.mergeable = MergeableState::Conflicting;
            rotted.checks = vec![check("SUCCESS")?];

            let mut broken = open_pr(3, "octocat")?;
            broken.checks = vec![check("FAILURE")?];

            let mut asked = open_pr(4, "hubot")?;
            asked.requested_reviewers = vec![Reviewer::User(me.clone())];
            asked.checks = vec![check("SUCCESS")?];

            // Somebody else's, failing, and none of the viewer's business.
            let mut theirs = open_pr(5, "hubot")?;
            theirs.checks = vec![check("FAILURE")?];

            let waiting = already_waiting(
                &judged(vec![mergeable, rotted, broken, asked, theirs], &policy),
                &me,
            );
            Ok::<_, InvalidValue>(waiting.describe())
        });

        assert_eq!(
            counted,
            Ok(Some(
                "1 ready to merge, 1 approved but conflicting, 1 with a required check failing, \
                 1 waiting on your review"
                    .to_owned()
            ))
        );
    }

    #[test]
    fn a_board_with_nothing_to_act_on_says_so_by_saying_nothing() {
        // `None` rather than "0 ready to merge, 0 ...", so the caller can
        // leave the clause out entirely instead of printing a row of zeroes.
        let counted = Login::parse("octocat").and_then(|me| {
            let policy = rule()?;
            let mut running = open_pr(1, "octocat")?;
            running.checks = vec![check("IN_PROGRESS")?];
            Ok::<_, InvalidValue>(already_waiting(&judged(vec![running], &policy), &me).describe())
        });

        assert_eq!(counted, Ok(None));
    }

    #[test]
    fn somebody_elses_broken_check_is_not_something_you_can_act_on() {
        // The count is about what the *viewer* can do. Somebody else's failing
        // required check is their problem, and counting it would inflate the
        // one number the startup line exists to make trustworthy.
        let counted = Login::parse("octocat").and_then(|me| {
            let policy = rule()?;
            let mut theirs = open_pr(1, "hubot")?;
            theirs.checks = vec![check("FAILURE")?];
            Ok::<_, InvalidValue>(already_waiting(&judged(vec![theirs], &policy), &me).describe())
        });

        assert_eq!(counted, Ok(None));
    }

    #[test]
    fn a_nudged_pull_request_reads_as_one_line() {
        let line = Login::parse("octocat").and_then(|me| {
            let policy = rule()?;
            let mut before = open_pr(7, "octocat")?;
            before.checks = vec![check("IN_PROGRESS")?];
            let mut after = before.clone();
            after.review_decision = Some(ReviewDecision::Approved);
            after.checks = vec![check("SUCCESS")?];

            let mut state = WatchState::default();
            let _baseline = tick(&mut state, &judged(vec![before], &policy), &me);
            let nudged = tick(&mut state, &judged(vec![after], &policy), &me);
            Ok::<_, InvalidValue>(nudged.first().map(Nudged::summary))
        });

        assert_eq!(line, Ok(Some("#7 is ready to merge".to_owned())));
    }
}
