//! Why a pull request cannot merge yet — one answer, from a fixed ladder.
//!
//! Everything else in this crate exists to feed [`BlockedReason::decide`] or to
//! render what it returns. A pull request is usually wrong in several ways at
//! once, and a board that lists all of them is a list of things not to do. The
//! ladder picks the one that is actually in the way.

use std::num::{NonZeroU32, NonZeroUsize};

use crate::checks::classify;
use crate::facts::{DraftState, MergeState, MergeableState, PrFacts, ReviewDecision, Reviewer};
use crate::policy::{PolicyRules, Requirement};
use crate::types::Login;

/// How many of a pull request's checks are failing in a way that holds the
/// merge back — never zero.
///
/// A count, not a list, because the names are already on the board in the CI
/// column and repeating them in the reason would say the same thing twice.
///
/// Zero is unrepresentable because zero is a different rung: a pull request
/// with no blocking failure is not blocked on CI at all, and a reason reading
/// "0 required checks failing" would send someone to look at a green build.
///
/// It is a named type rather than a bare number so that it cannot be confused
/// with the other count on this ladder, [`UnresolvedThreadCount`]. The two are
/// both "how many things are wrong", they sit on adjacent rungs, and a bare
/// integer would let a construction site or a renderer swap them silently.
///
/// # Examples
///
/// ```
/// use prtrack_core::FailingCheckCount;
///
/// assert_eq!(FailingCheckCount::new(2).map(FailingCheckCount::get), Some(2));
/// assert_eq!(FailingCheckCount::new(0), None);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailingCheckCount(NonZeroUsize);

impl FailingCheckCount {
    /// Holds a count of blocking failures, or reports that there were none.
    ///
    /// `None` is not an error; it is the answer that this pull request is not
    /// on this rung.
    #[must_use]
    pub const fn new(count: usize) -> Option<Self> {
        match NonZeroUsize::new(count) {
            Some(count) => Some(Self(count)),
            None => None,
        }
    }

    /// The count, for rendering.
    #[must_use]
    pub const fn get(self) -> usize {
        self.0.get()
    }
}

/// How many review threads are still open on a pull request whose repository
/// demands they be resolved — never zero.
///
/// Distinct from [`FailingCheckCount`] for the reason given there: the two are
/// adjacent rungs of the same ladder, both count things that are wrong, and a
/// bare integer would let them be swapped without the compiler noticing.
///
/// Zero is unrepresentable because a pull request with no open thread is not on
/// this rung. Note that this is a count of *unresolved* threads only — whether
/// they block at all is [`PolicyRules::conversation_resolution`], which the
/// ladder consults before it ever builds one of these.
///
/// # Examples
///
/// ```
/// use prtrack_core::UnresolvedThreadCount;
///
/// assert_eq!(UnresolvedThreadCount::new(3).map(UnresolvedThreadCount::get), Some(3));
/// assert_eq!(UnresolvedThreadCount::new(0), None);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnresolvedThreadCount(NonZeroU32);

impl UnresolvedThreadCount {
    /// Holds a count of open threads, or reports that there were none.
    ///
    /// `None` is not an error; it is the answer that this pull request is not
    /// on this rung.
    #[must_use]
    pub const fn new(count: u32) -> Option<Self> {
        match NonZeroU32::new(count) {
            Some(count) => Some(Self(count)),
            None => None,
        }
    }

    /// The count, for rendering.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

/// A list that cannot be empty.
///
/// Several rungs of the ladder name the people a pull request is waiting on,
/// and every one of them is only that rung *because* the list is not empty: a
/// pull request whose blockers are nobody is not blocked by a review, and one
/// whose reviewers are nobody is [`BlockedReason::NobodyAssigned`]. Holding a
/// plain `Vec` would leave "changes requested by nobody" expressible, and a
/// renderer would print the empty sentence rather than fail.
///
/// The type parameter also keeps two such lists apart: a `NonEmpty<Login>` will
/// not compile where a `NonEmpty<Reviewer>` is wanted, however alike the two
/// read at a call site.
///
/// # Examples
///
/// ```
/// use prtrack_core::{Login, NonEmpty};
///
/// let blockers = NonEmpty::new(vec![Login::parse("hubot")?]);
/// assert_eq!(blockers.map(|by| by.as_slice().len()), Some(1));
/// assert_eq!(NonEmpty::new(Vec::<Login>::new()), None);
/// # Ok::<(), prtrack_core::InvalidValue>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonEmpty<T>(Vec<T>);

impl<T> NonEmpty<T> {
    /// Holds `items`, or reports that there were none.
    ///
    /// `None` is not an error. An empty list is a legitimate fact about a pull
    /// request — nobody asked for changes — it simply is not the fact this type
    /// exists to carry, and the ladder reads it as a different rung.
    #[must_use]
    pub fn new(items: Vec<T>) -> Option<Self> {
        (!items.is_empty()).then_some(Self(items))
    }

    /// The items, in the order they arrived, for rendering and for counting.
    ///
    /// Never empty, so `.len()` is at least one and `.first()` is always
    /// `Some` — but the slice is a plain slice, so a caller reading it still
    /// has to say what it wants from an `Option`. That is the seam where the
    /// guarantee stops being carried by the type.
    #[must_use]
    pub fn as_slice(&self) -> &[T] {
        &self.0
    }
}

/// The one reason a pull request cannot merge yet.
///
/// An enum, not a sentence: deciding and rendering are separate concerns, and a
/// caller that has to parse prose back out of a cell cannot sort by it, count
/// it, or nudge on it. The wording lives in the shell; the variants live here
/// and are what a section order and a notification are keyed on.
///
/// # One reason, not a list
///
/// A stuck pull request is normally wrong in several ways at once — a red
/// check, an open thread, and a reviewer who has not answered. Reporting all
/// three describes the situation without saying what to do about it, so
/// [`BlockedReason::decide`] returns the first rung that matches and stops.
/// Where two conditions are both true, the higher rung is the one that has to
/// move first: the check will be re-run by the next push anyway, and the review
/// will not answer itself.
///
/// # Matching stays exhaustive
///
/// This enum is not `#[non_exhaustive]`, so a renderer's match over it needs no
/// wildcard arm and a rung added later breaks every consumer that has to decide
/// what the new reason should say. That is the point: a rung that fell into a
/// catch-all would render as whatever the catch-all says, which is the wrong
/// cell wearing a plausible face.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockedReason {
    /// The author has not asked for review yet.
    Draft,
    /// A reviewer is holding the pull request back, and these are the accounts
    /// that asked for changes.
    ChangesRequested {
        /// Who asked, in the order GitHub reported them. Never empty.
        by: NonEmpty<Login>,
    },
    /// A check the policy says can hold the merge back is failing.
    RequiredCheckFailing {
        /// How many. Never zero.
        count: FailingCheckCount,
    },
    /// Review threads are still open and the repository will not merge until
    /// they are resolved.
    UnresolvedThreads {
        /// How many. Never zero.
        count: UnresolvedThreadCount,
    },
    /// It was approved, and then the base branch moved under it.
    ///
    /// The case the board exists to catch: work that passed review and quietly
    /// rotted. Nobody is watching it — the reviewer is finished and the author
    /// believes it is done — so it stays there until something says otherwise.
    ApprovedButConflicting,
    /// It conflicts with its base and nobody has approved it yet.
    ///
    /// Distinct from [`ApprovedButConflicting`] because the sentence is
    /// different: that one is "you were approved, rebase and get re-approved",
    /// this one is "rebase before anyone spends time reviewing it". A conflict
    /// resolution changes the diff, so a review given now is a review of code
    /// that is about to be rewritten — and where the rule dismisses stale
    /// reviews, the push throws the approval away as well.
    ///
    /// [`ApprovedButConflicting`]: BlockedReason::ApprovedButConflicting
    /// [`Conflicting`]: BlockedReason::Conflicting
    /// [`BehindBase`]: BlockedReason::BehindBase
    /// [`MergeabilityNotKnown`]: BlockedReason::MergeabilityNotKnown
    Conflicting,
    /// The base branch has moved on and the rule will not merge a branch that
    /// is not current.
    ///
    /// Not a conflict — the two sides do not disagree, this branch is simply
    /// older than its base. It is only a reason where
    /// [`PolicyRules::up_to_date_branch`] says so, exactly as open threads are
    /// only a reason where the rule demands resolution. Where it is set,
    /// GitHub disables the merge button until the branch is updated.
    BehindBase,
    /// It was approved and CI has not finished.
    ApprovedCiRunning,
    /// Nothing is in the way. Somebody should press the button.
    ReadyToMerge,
    /// CI has not finished, and nothing above it is wrong.
    ///
    /// There is no timeout on this rung. A check reporting a terminal state
    /// this crate does not recognise classifies as running, and no later
    /// attempt is coming, so it will read as "CI running" indefinitely. That is
    /// a known gap watched by `scripts/check-states.sh`, not a promise that
    /// this state is transient.
    CiRunning,
    /// Somebody was asked to review it and has not answered.
    WaitingForReview {
        /// Who was asked, in the order GitHub reported them. Never empty.
        ///
        /// A [`Reviewer::Team`] names no account and a
        /// [`Reviewer::Unknown`](crate::Reviewer::Unknown) names nothing at
        /// all, but both are real requests and both are kept: dropping one
        /// would shorten the list into [`BlockedReason::NobodyAssigned`] and
        /// tell a team to find a reviewer they already have.
        from: NonEmpty<Reviewer>,
    },
    /// GitHub has not said whether this merges cleanly.
    ///
    /// Covers both `mergeable: UNKNOWN`, which is a background computation that
    /// has not finished, and a value this crate could not read. Neither is a
    /// conflict and neither is a clean merge, so the honest answer is that the
    /// question is open and the honest next step is to ask again.
    MergeabilityNotKnown,
    /// Nobody has been asked to look at it.
    NobodyAssigned,
}

impl BlockedReason {
    /// Reads the one reason `facts` cannot merge under `policy`.
    ///
    /// First match wins, top to bottom:
    ///
    /// | # | Rung | Reached when |
    /// | --- | --- | --- |
    /// | 1 | [`Draft`] | the author has not asked for review |
    /// | 2 | [`ChangesRequested`] | somebody's review is holding it back |
    /// | 3 | [`RequiredCheckFailing`] | a failure the policy says can block |
    /// | 4 | [`UnresolvedThreads`] | threads are open *and* the rule demands resolution |
    /// | 5 | [`ApprovedButConflicting`] | approved, then the base moved under it |
    /// | 6 | [`Conflicting`] | conflicts, and no approval to lose |
    /// | 7 | [`BehindBase`] | approved, out of date, *and* the rule demands a current branch |
    /// | 8 | [`ApprovedCiRunning`] | approved, CI still going |
    /// | 9 | [`ReadyToMerge`] | approved, merge confirmed clean, nothing left |
    /// | 10 | [`CiRunning`] | not approved, CI still going |
    /// | 11 | [`MergeabilityNotKnown`] | GitHub has not said whether it merges |
    /// | 12 | [`WaitingForReview`] | somebody was asked and has not answered |
    /// | 13 | [`NobodyAssigned`] | nobody was asked |
    ///
    /// # Which of these outrank a pending review, and why they differ
    ///
    /// [`Conflicting`] and [`MergeabilityNotKnown`] sit above
    /// [`WaitingForReview`]. [`BehindBase`] deliberately does not: it lives
    /// inside the approval, so it is reached only when it is the last thing
    /// between a pull request and the merge button.
    ///
    /// The distinction is what the author's next action does to the diff.
    /// Resolving a conflict **rewrites it**, so a review given first is a
    /// review of code that will not exist — and where the rule dismisses stale
    /// reviews, the push throws the approval away too. Updating a branch that
    /// is merely behind changes **not one line** of what the author wrote; the
    /// reviewer reads the same diff either way, and the only cost is a
    /// re-click. The two look alike and are not.
    ///
    /// The cost of getting it wrong is asymmetric. GitHub reports `BEHIND` only
    /// where strict status checks are enabled, and where they are, a pull
    /// request falls behind every time its base moves. Ranking it above the
    /// review request would therefore mask that request for most pull requests
    /// most of the time, on exactly the repositories that turn the flag on.
    /// The interruption should name the bottleneck, and before approval the
    /// bottleneck is the reviewer.
    ///
    /// # It takes the policy, not a classification
    ///
    /// The checks are classified here, from `policy`, rather than accepted
    /// pre-classified. A [`Classified`] built against a different rule — or
    /// against a different pull request — would compile and give a confident
    /// wrong answer, and there is no way to check after the fact which rule it
    /// came from. A caller that also needs the buckets for a CI column calls
    /// [`classify`] itself; it is a pure pass over a few dozen values.
    ///
    /// Three policy fields gate rungs: [`PolicyRules::blocking_checks`] decides
    /// which failures reach rung 3, [`PolicyRules::conversation_resolution`]
    /// gates rung 4, and [`PolicyRules::up_to_date_branch`] gates rung 7. The
    /// fields a policy is *not* consulted about are worth naming.
    /// [`PolicyRules::required_approvals`] is never read, because GitHub has
    /// already folded the approval count into
    /// [`PrFacts::review_decision`](crate::PrFacts::review_decision). And rung 4
    /// reads the conversation rule of the policy it is handed, which for a pull
    /// request aimed at an unprotected branch is
    /// [`PolicyRules::unprotected`] — so open threads on a stacked pull request
    /// are informational, as the branch it targets does not enforce them.
    ///
    /// # No clock
    ///
    /// Nothing here is time-dependent, so there is no `now` parameter. In
    /// particular [`CiRunning`] has no timeout: see the variant.
    ///
    /// # Examples
    ///
    /// ```
    /// use prtrack_core::{BlockedReason, PolicyRules, PrFacts};
    /// # fn demo(finished: &PrFacts, rule: &PolicyRules) {
    /// match BlockedReason::decide(finished, rule) {
    ///     BlockedReason::ReadyToMerge => println!("go and press the button"),
    ///     other => println!("not yet: {other:?}"),
    /// }
    /// # }
    /// ```
    ///
    /// [`Draft`]: BlockedReason::Draft
    /// [`ChangesRequested`]: BlockedReason::ChangesRequested
    /// [`RequiredCheckFailing`]: BlockedReason::RequiredCheckFailing
    /// [`UnresolvedThreads`]: BlockedReason::UnresolvedThreads
    /// [`ApprovedButConflicting`]: BlockedReason::ApprovedButConflicting
    /// [`Conflicting`]: BlockedReason::Conflicting
    /// [`BehindBase`]: BlockedReason::BehindBase
    /// [`MergeabilityNotKnown`]: BlockedReason::MergeabilityNotKnown
    /// [`ApprovedCiRunning`]: BlockedReason::ApprovedCiRunning
    /// [`ReadyToMerge`]: BlockedReason::ReadyToMerge
    /// [`CiRunning`]: BlockedReason::CiRunning
    /// [`WaitingForReview`]: BlockedReason::WaitingForReview
    /// [`NobodyAssigned`]: BlockedReason::NobodyAssigned
    /// [`Classified`]: crate::Classified
    /// [`classify`]: crate::classify
    #[must_use]
    pub fn decide(facts: &PrFacts, policy: &PolicyRules) -> Self {
        let checks = classify(&facts.checks, policy);

        match facts.draft_state {
            DraftState::Draft => return Self::Draft,
            DraftState::ReadyForReview => {}
        }

        if let Some(by) = NonEmpty::new(facts.changes_requested_by.clone()) {
            return Self::ChangesRequested { by };
        }

        if let Some(count) = FailingCheckCount::new(checks.blocking().len()) {
            return Self::RequiredCheckFailing { count };
        }

        match policy.conversation_resolution {
            Requirement::Required => {
                if let Some(count) = UnresolvedThreadCount::new(facts.unresolved_threads) {
                    return Self::UnresolvedThreads { count };
                }
            }
            Requirement::NotRequired => {}
        }

        let approved = is_approved(facts.review_decision.as_ref());
        let ci_running = !checks.running().is_empty();

        // One conflict decision, branching only on what the sentence should
        // say. Two separate rungs at different heights would let a conflict
        // outrank a review request when approved and lose to it when not,
        // which is backwards: the unapproved case is the one where reviewer
        // time is about to be wasted.
        let readiness = merge_readiness(&facts.mergeable);

        match readiness {
            MergeReadiness::Conflicting => {
                return if approved {
                    Self::ApprovedButConflicting
                } else {
                    Self::Conflicting
                }
            }
            MergeReadiness::Clean | MergeReadiness::NotYetKnown => {}
        }

        if approved {
            // Inside the approval on purpose. Being behind is only worth saying
            // when it is the last thing between this pull request and the
            // button — before that the bottleneck is the reviewer, and updating
            // a branch does not change a line of the diff they would read.
            match policy.up_to_date_branch {
                Requirement::Required => {
                    if is_behind(&facts.merge_state) {
                        return Self::BehindBase;
                    }
                }
                Requirement::NotRequired => {}
            }
            if ci_running {
                return Self::ApprovedCiRunning;
            }
            match readiness {
                MergeReadiness::Clean => return Self::ReadyToMerge,
                MergeReadiness::Conflicting | MergeReadiness::NotYetKnown => {}
            }
        }

        if ci_running {
            return Self::CiRunning;
        }

        match readiness {
            MergeReadiness::NotYetKnown => return Self::MergeabilityNotKnown,
            MergeReadiness::Clean | MergeReadiness::Conflicting => {}
        }

        NonEmpty::new(facts.requested_reviewers.clone())
            .map_or(Self::NobodyAssigned, |from| Self::WaitingForReview { from })
    }
}

/// Whether GitHub's own review summary says the review requirement is met.
///
/// Written as a match that names every other value rather than as an equality
/// test, so that a decision GitHub adds later stops this compiling instead of
/// silently joining the not-approved side. Absence is one of those values and
/// is deliberately not an approval: `null` means the repository requires no
/// review, and reading that as "approved" would tell someone to merge work
/// nobody has looked at.
const fn is_approved(decision: Option<&ReviewDecision>) -> bool {
    match decision {
        Some(ReviewDecision::Approved) => true,
        Some(
            ReviewDecision::ChangesRequested
            | ReviewDecision::ReviewRequired
            | ReviewDecision::Unknown { .. },
        )
        | None => false,
    }
}

/// Whether the base branch has moved on since this branch last took from it.
///
/// Read from `mergeStateStatus`, which is the only fact in [`PrFacts`] that
/// reports it. GitHub only returns `BEHIND` where the rule requires a current
/// branch, so this and [`PolicyRules::up_to_date_branch`] normally agree — but
/// they arrive from separate reads and the ladder checks both rather than
/// trusting one to imply the other.
const fn is_behind(state: &MergeState) -> bool {
    match state {
        MergeState::Behind => true,
        MergeState::Blocked
        | MergeState::Clean
        | MergeState::Dirty
        | MergeState::Draft
        | MergeState::HasHooks
        | MergeState::Unstable
        | MergeState::Undetermined
        | MergeState::Unknown { .. } => false,
    }
}

/// What git's view of the merge means for the ladder.
///
/// Three states rather than the four [`MergeableState`] has, because the two
/// GitHub has not answered lead to the same rung. Deliberately not public: a
/// caller wants the reason, not a re-derivation of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MergeReadiness {
    /// GitHub confirmed the merge is clean.
    Clean,
    /// The branch and its base have diverged in the same places.
    Conflicting,
    /// GitHub has not said. Not a conflict, and not clean either.
    NotYetKnown,
}

/// Reads git's view of the merge, folding both non-answers together.
///
/// [`MergeableState::Computing`] is not a conflict — GitHub works mergeability
/// out in the background, and treating "not yet known" as "conflicting" would
/// invent a rebase for an author whose branch is fine. Nor is it clean, which
/// is the other half of the same point: only a confirmed `MERGEABLE` earns
/// [`BlockedReason::ReadyToMerge`], because "go and press the button" is the
/// expensive thing to say on a guess. [`MergeableState::Unknown`] joins it for
/// the same reason — a value this crate could not read is not a clean merge.
const fn merge_readiness(mergeable: &MergeableState) -> MergeReadiness {
    match mergeable {
        MergeableState::Mergeable => MergeReadiness::Clean,
        MergeableState::Conflicting => MergeReadiness::Conflicting,
        MergeableState::Computing | MergeableState::Unknown { .. } => MergeReadiness::NotYetKnown,
    }
}

#[cfg(test)]
mod tests {
    use super::{BlockedReason, FailingCheckCount, NonEmpty, UnresolvedThreadCount};
    use crate::checks::{Check, CheckState};
    use crate::facts::{
        BranchName, DraftState, MergeState, MergeableState, PrFacts, PrTitle, PrUrl,
        ReviewDecision, Reviewer, TeamSlug,
    };
    use crate::policy::{BlockingChecks, PolicyRules, Requirement};
    use crate::types::{CheckName, HeadSha, InvalidValue, Login, PrNumber, RepoRef};
    use chrono::{DateTime, Utc};

    /// A check `Example-Org/example-repo` requires on `main`, read from the live
    /// rule. Failing it holds the merge back.
    const REQUIRED: &str = "Preview CI Gate";

    /// A second required check, so a test can tell one blocker from two.
    const REQUIRED_TITLE: &str = "Validate PR Title";

    /// A check no rule names. It runs on every pull request and the merge
    /// button ignores it however red it goes.
    const ADVISORY: &str = "codecov/patch";

    /// A second advisory check, so "none of these block" is not carried by a
    /// single row.
    const ADVISORY_BUILD: &str = "build / test (ubuntu-latest)";

    /// A rule that names exactly `required`, demands nothing else, and does not
    /// enforce thread resolution.
    ///
    /// A context that does not parse yields `Err` rather than a rule that
    /// quietly requires nothing — under which every "this check blocks"
    /// assertion would still pass, by way of the `Every` fallback.
    fn rule_requiring(required: &[&str]) -> Result<PolicyRules, InvalidValue> {
        let contexts: Vec<CheckName> = required
            .iter()
            .map(|name| CheckName::parse(name))
            .collect::<Result<_, _>>()?;
        Ok(PolicyRules {
            blocking_checks: BlockingChecks::from_rule(Requirement::Required, contexts),
            ..PolicyRules::unprotected()
        })
    }

    /// The same rule, with the repository's conversation-resolution setting
    /// switched on.
    fn requiring_resolution(policy: PolicyRules) -> PolicyRules {
        PolicyRules {
            conversation_resolution: Requirement::Required,
            ..policy
        }
    }

    /// Who the reason says is holding the pull request back, or the rung that
    /// was reached instead.
    ///
    /// The `Err` side carries the whole reason on purpose, so a test that lands
    /// on the wrong rung prints which one rather than a bare inequality. These
    /// projections are written as `if let` rather than as an exhaustive match:
    /// a rung added later does not change what any of them should extract, and
    /// [`rung`] is the helper that must stop compiling when one is.
    fn changes_requested_by(reason: BlockedReason) -> Result<Vec<String>, BlockedReason> {
        if let BlockedReason::ChangesRequested { by } = &reason {
            Ok(by
                .as_slice()
                .iter()
                .map(|login| login.as_str().to_owned())
                .collect())
        } else {
            Err(reason)
        }
    }

    /// The same rule, with the repository's strict-status-checks setting
    /// switched on, which is what makes an out-of-date branch unmergeable.
    fn demanding_a_current_branch(policy: PolicyRules) -> PolicyRules {
        PolicyRules {
            up_to_date_branch: Requirement::Required,
            ..policy
        }
    }

    /// How many required checks the reason says are failing, or the rung that
    /// was reached instead.
    fn failing_checks(reason: BlockedReason) -> Result<usize, BlockedReason> {
        if let BlockedReason::RequiredCheckFailing { count } = &reason {
            Ok(count.get())
        } else {
            Err(reason)
        }
    }

    /// How many open threads the reason says are in the way, or the rung that
    /// was reached instead.
    fn unresolved_threads(reason: BlockedReason) -> Result<u32, BlockedReason> {
        if let BlockedReason::UnresolvedThreads { count } = &reason {
            Ok(count.get())
        } else {
            Err(reason)
        }
    }

    /// The rung a reason sits on, named, ignoring what it carries.
    ///
    /// The payloads are pinned by the per-rung tests; this exists for the tests
    /// about *order*, where naming the rung is the whole assertion. Exhaustive
    /// without a wildcard, so a rung added later stops this compiling rather
    /// than quietly reporting as one of the ten.
    fn rung(reason: &BlockedReason) -> &'static str {
        match reason {
            BlockedReason::Draft => "draft",
            BlockedReason::ChangesRequested { .. } => "changes-requested",
            BlockedReason::RequiredCheckFailing { .. } => "required-check-failing",
            BlockedReason::UnresolvedThreads { .. } => "unresolved-threads",
            BlockedReason::ApprovedButConflicting => "approved-but-conflicting",
            BlockedReason::Conflicting => "conflicting",
            BlockedReason::BehindBase => "behind-base",
            BlockedReason::ApprovedCiRunning => "approved-ci-running",
            BlockedReason::ReadyToMerge => "ready-to-merge",
            BlockedReason::CiRunning => "ci-running",
            BlockedReason::MergeabilityNotKnown => "mergeability-not-known",
            BlockedReason::WaitingForReview { .. } => "waiting-for-review",
            BlockedReason::NobodyAssigned => "nobody-assigned",
        }
    }

    /// Who the reason says review is awaited from, or the rung that was reached
    /// instead.
    ///
    /// `None` inside the list is a reviewer of a kind this crate does not
    /// model, which has no name to render. Keeping it as a row rather than
    /// dropping it is what proves the request was not silently discarded.
    fn waiting_for_review(reason: BlockedReason) -> Result<Vec<Option<String>>, BlockedReason> {
        if let BlockedReason::WaitingForReview { from } = &reason {
            Ok(from
                .as_slice()
                .iter()
                .map(|reviewer| reviewer.name().map(str::to_owned))
                .collect())
        } else {
            Err(reason)
        }
    }

    /// A pull request with nothing yet decided about it: open for review, no
    /// review given, mergeable, no checks reported, no unresolved threads, and
    /// nobody asked to look at it.
    ///
    /// It sits on the last rung of the ladder on purpose. Every test below
    /// reaches its rung by changing only the facts that rung reads, so a test
    /// that passed for some other reason would have to have changed a fact it
    /// did not name.
    ///
    /// Built through the real constructors, so a fixture that stopped parsing
    /// yields `Err` rather than leaving the assertions quietly asserting
    /// nothing.
    fn open_pull_request() -> Result<PrFacts, InvalidValue> {
        Ok(PrFacts {
            repo: RepoRef::parse("Example-Org/example-repo")?,
            number: PrNumber::new(4225)?,
            url: PrUrl::new("https://github.com/Example-Org/example-repo/pull/4225"),
            title: PrTitle::new("Tighten the ledger writer"),
            author: Some(Login::parse("octocat")?),
            draft_state: DraftState::ReadyForReview,
            head: HeadSha::parse("9f2c4e1b7a3d5068c9e4b2a17f36d8095c1e4a7b")?,
            head_ref: BranchName::new("topic/branch-01"),
            base_ref: BranchName::new("main"),
            review_decision: Some(ReviewDecision::ReviewRequired),
            mergeable: MergeableState::Mergeable,
            merge_state: MergeState::Blocked,
            requested_reviewers: Vec::new(),
            changes_requested_by: Vec::new(),
            reviewed_by: Vec::new(),
            unresolved_threads: 0,
            comments: 0,
            checks: Vec::new(),
            last_push: DateTime::<Utc>::UNIX_EPOCH,
        })
    }

    /// The rollup a pull request reported, as `(name, state)` pairs.
    ///
    /// A name that does not parse yields `Err` for the whole fixture rather
    /// than a rollup quietly one check short — under which a test about a
    /// failing check would pass against an empty list.
    fn reported(rows: &[(&str, &str)]) -> Result<Vec<Check>, InvalidValue> {
        rows.iter()
            .map(|(name, state)| {
                Ok(Check::new(
                    CheckName::parse(name)?,
                    CheckState::parse(state),
                    None,
                ))
            })
            .collect()
    }

    /// Decides one pull request: the baseline, with `change` applied.
    ///
    /// `change` returns a `Result` so that a login or a check name it has to
    /// build stays inside the same `Result` as the verdict, and a parse failure
    /// can never be mistaken for a rung.
    fn decided(
        policy: &PolicyRules,
        change: impl FnOnce(&mut PrFacts) -> Result<(), InvalidValue>,
    ) -> Result<BlockedReason, InvalidValue> {
        let mut facts = open_pull_request()?;
        change(&mut facts)?;
        Ok(BlockedReason::decide(&facts, policy))
    }

    #[test]
    fn a_draft_is_not_asking_for_anything_yet() {
        assert_eq!(
            decided(&PolicyRules::unprotected(), |pr| {
                pr.draft_state = DraftState::Draft;
                Ok(())
            }),
            Ok(BlockedReason::Draft),
            "an author who has not asked for review is the first rung"
        );
    }

    #[test]
    fn a_pull_request_nobody_was_asked_to_look_at_is_waiting_on_nobody() {
        assert_eq!(
            decided(&PolicyRules::unprotected(), |_| Ok(())),
            Ok(BlockedReason::NobodyAssigned),
            "green, mergeable, and unrequested is the last rung, not the first"
        );
    }

    #[test]
    fn a_reviewer_holding_the_pull_request_back_is_named() {
        // Keyed on who actually asked for changes, not on GitHub's summary
        // decision: the summary says only *that* someone is holding it back,
        // and a reason that cannot name them tells the author nothing about
        // whose door to knock on.
        assert_eq!(
            decided(&PolicyRules::unprotected(), |pr| {
                pr.review_decision = Some(ReviewDecision::ChangesRequested);
                pr.changes_requested_by = vec![Login::parse("hubot")?, Login::parse("dependabot")?];
                Ok(())
            })
            .map(changes_requested_by),
            Ok(Ok(vec!["hubot".to_owned(), "dependabot".to_owned()])),
            "every reviewer holding it back must survive to the reason, in the order given"
        );
    }

    #[test]
    fn a_reason_cannot_say_changes_were_requested_by_nobody() {
        assert_eq!(
            NonEmpty::new(Vec::<Login>::new()),
            None,
            "an empty list of blockers is not a request for changes"
        );
    }

    #[test]
    fn a_failing_check_the_rule_names_is_what_the_merge_is_waiting_on() {
        assert_eq!(
            rule_requiring(&[REQUIRED, REQUIRED_TITLE])
                .and_then(|policy| decided(&policy, |pr| {
                    pr.checks = reported(&[
                        (REQUIRED, "FAILURE"),
                        (REQUIRED_TITLE, "TIMED_OUT"),
                        (ADVISORY, "FAILURE"),
                    ])?;
                    Ok(())
                }))
                .map(failing_checks),
            Ok(Ok(2)),
            "the two required failures are the blockers; the advisory one is not counted"
        );
    }

    #[test]
    fn a_failing_check_the_rule_does_not_name_never_reaches_that_rung() {
        // The distinction the whole board exists for. On the target repository
        // the median pull request runs 36 checks and four of them can block, so
        // painting every red one as a blocker blames an author for a check the
        // merge button ignores.
        assert_eq!(
            rule_requiring(&[REQUIRED]).and_then(|policy| decided(&policy, |pr| {
                pr.checks = reported(&[(ADVISORY, "FAILURE"), (ADVISORY_BUILD, "ERROR")])?;
                Ok(())
            })),
            Ok(BlockedReason::NobodyAssigned),
            "a red check no rule names cannot hold a merge, so it is not the reason"
        );
    }

    #[test]
    fn with_no_rule_to_read_any_failing_check_is_what_the_merge_is_waiting_on() {
        // The stacked-pull-request case. Silence in the policy means "we cannot
        // tell", not "nothing matters", so the fallback treats every failure as
        // blocking. There is deliberately no API listing the required checks in
        // general, so this rung must not be derived from an empty set.
        assert_eq!(
            decided(&PolicyRules::unprotected(), |pr| {
                pr.checks = reported(&[(ADVISORY, "FAILURE")])?;
                Ok(())
            })
            .map(failing_checks),
            Ok(Ok(1)),
            "with no rule to read, an unnamed failure still holds the merge back"
        );
    }

    #[test]
    fn a_reason_cannot_say_zero_required_checks_are_failing() {
        assert_eq!(
            FailingCheckCount::new(0),
            None,
            "no failing check is not a rung, it is the absence of one"
        );
        assert_eq!(
            FailingCheckCount::new(3).map(FailingCheckCount::get),
            Some(3),
            "a count that survives must be the count that went in"
        );
    }

    #[test]
    fn an_unresolved_thread_blocks_only_where_the_repository_enforces_resolution() {
        assert_eq!(
            rule_requiring(&[REQUIRED])
                .map(requiring_resolution)
                .and_then(|policy| decided(&policy, |pr| {
                    pr.unresolved_threads = 3;
                    Ok(())
                }))
                .map(unresolved_threads),
            Ok(Ok(3)),
            "where the rule demands resolution, an open thread is what the merge waits on"
        );
    }

    #[test]
    fn an_unresolved_thread_is_skipped_entirely_where_the_rule_does_not_demand_resolution() {
        // Not a lower rung — no rung at all. The threads are still there and
        // still worth reading, but they cannot hold the merge, so naming them
        // as the reason would send someone to resolve a conversation that was
        // never in the way.
        assert_eq!(
            rule_requiring(&[REQUIRED]).and_then(|policy| decided(&policy, |pr| {
                pr.unresolved_threads = 3;
                Ok(())
            })),
            Ok(BlockedReason::NobodyAssigned),
            "a thread the rule does not require resolved is informational, not blocking"
        );
    }

    #[test]
    fn a_reason_cannot_say_zero_threads_are_unresolved() {
        assert_eq!(
            UnresolvedThreadCount::new(0),
            None,
            "no open thread is not a rung, it is the absence of one"
        );
        assert_eq!(
            UnresolvedThreadCount::new(7).map(UnresolvedThreadCount::get),
            Some(7),
            "a count that survives must be the count that went in"
        );
    }

    #[test]
    fn work_that_passed_review_and_then_rotted_is_the_case_the_board_exists_to_catch() {
        // Approved, and the base branch moved under it. Nobody is looking at
        // this pull request any more — the reviewer is done and the author
        // thinks it is finished — so it sits there until something says so.
        assert_eq!(
            rule_requiring(&[REQUIRED]).and_then(|policy| decided(&policy, |pr| {
                pr.review_decision = Some(ReviewDecision::Approved);
                pr.mergeable = MergeableState::Conflicting;
                pr.merge_state = MergeState::Dirty;
                Ok(())
            })),
            Ok(BlockedReason::ApprovedButConflicting),
            "an approval a conflict has invalidated is not a merge-ready pull request"
        );
    }

    #[test]
    fn an_approved_pull_request_with_ci_still_running_is_not_ready_to_merge() {
        assert_eq!(
            rule_requiring(&[REQUIRED]).and_then(|policy| decided(&policy, |pr| {
                pr.review_decision = Some(ReviewDecision::Approved);
                pr.checks = reported(&[(REQUIRED, "IN_PROGRESS"), (ADVISORY, "SUCCESS")])?;
                Ok(())
            })),
            Ok(BlockedReason::ApprovedCiRunning),
            "telling someone to merge before CI has spoken is the false nudge that costs most"
        );
    }

    #[test]
    fn an_approved_pull_request_with_nothing_left_to_wait_for_is_ready_to_merge() {
        assert_eq!(
            rule_requiring(&[REQUIRED]).and_then(|policy| decided(&policy, |pr| {
                pr.review_decision = Some(ReviewDecision::Approved);
                pr.merge_state = MergeState::Clean;
                pr.checks = reported(&[(REQUIRED, "SUCCESS"), (ADVISORY, "FAILURE")])?;
                Ok(())
            })),
            Ok(BlockedReason::ReadyToMerge),
            "the required check passed and only an advisory one is red, so nothing is in the way"
        );
    }

    #[test]
    fn a_conflicting_pull_request_needs_a_rebase_before_anyone_spends_time_reviewing_it() {
        // Live shape, `Example-Org/example-repo#4250`: conflicting, unapproved,
        // three people requested. Six open pull requests were in this state
        // when it was measured, and the board was sending those reviewers at
        // work that has to be rebased first. The repository also sets
        // `dismissesStaleReviews`, so the rebase would throw the review away.
        assert_eq!(
            rule_requiring(&[REQUIRED])
                .and_then(|policy| decided(&policy, |pr| {
                    pr.review_decision = Some(ReviewDecision::ReviewRequired);
                    pr.mergeable = MergeableState::Conflicting;
                    pr.merge_state = MergeState::Dirty;
                    pr.requested_reviewers = vec![
                        Reviewer::User(Login::parse("octocat")?),
                        Reviewer::User(Login::parse("hubot")?),
                        Reviewer::User(Login::parse("monalisa")?),
                    ];
                    Ok(())
                }))
                .as_ref()
                .map(rung),
            Ok("conflicting"),
            "an unapproved conflict is author work, and must outrank the review it would waste"
        );
    }

    #[test]
    fn the_two_conflict_rungs_differ_only_by_whether_an_approval_is_about_to_be_lost() {
        // One conflict decision, two sentences. Approved means "you were
        // approved, rebase and get re-approved"; unapproved means "rebase
        // before anyone spends time reviewing it". Nothing else about the pull
        // request differs between these two rows.
        let conflicted = |approved: Option<ReviewDecision>| {
            rule_requiring(&[REQUIRED])
                .and_then(|policy| {
                    decided(&policy, |pr| {
                        pr.review_decision = approved;
                        pr.mergeable = MergeableState::Conflicting;
                        pr.merge_state = MergeState::Dirty;
                        Ok(())
                    })
                })
                .map(|reason| rung(&reason))
        };

        assert_eq!(
            (
                conflicted(Some(ReviewDecision::Approved)),
                conflicted(Some(ReviewDecision::ReviewRequired)),
            ),
            (Ok("approved-but-conflicting"), Ok("conflicting")),
            "the approval is the only thing that chooses between the two conflict rungs"
        );
    }

    #[test]
    fn a_decision_that_is_not_an_approval_never_reaches_the_approved_rungs() {
        // Every other value `reviewDecision` can hold, including absence.
        // `None` means the repository requires no review at all — which is not
        // the same as having been approved, and reading it as one would tell
        // someone to merge work nobody has looked at.
        for decision in [
            None,
            Some(ReviewDecision::ReviewRequired),
            Some(ReviewDecision::parse("SOMETHING_NEW")),
        ] {
            assert_eq!(
                rule_requiring(&[REQUIRED]).and_then(|policy| decided(&policy, |pr| {
                    pr.review_decision = decision.clone();
                    pr.merge_state = MergeState::Clean;
                    pr.checks = reported(&[(REQUIRED, "SUCCESS")])?;
                    Ok(())
                })),
                Ok(BlockedReason::NobodyAssigned),
                "{decision:?} is not an approval, so it cannot reach the ready-to-merge rung"
            );
        }
    }

    #[test]
    fn an_unapproved_pull_request_still_running_ci_is_waiting_on_the_build() {
        // Advisory or required makes no difference here: a check that has not
        // reported has not delivered a verdict either way, so the policy has
        // nothing to apply yet.
        for still_going in [REQUIRED, ADVISORY] {
            assert_eq!(
                rule_requiring(&[REQUIRED]).and_then(|policy| decided(&policy, |pr| {
                    pr.checks = reported(&[(still_going, "QUEUED")])?;
                    pr.requested_reviewers = vec![Reviewer::User(Login::parse("hubot")?)];
                    Ok(())
                })),
                Ok(BlockedReason::CiRunning),
                "{still_going:?} has not reported, so the build is what is being waited on"
            );
        }
    }

    #[test]
    fn a_check_state_this_crate_cannot_read_leaves_the_pull_request_waiting_on_ci() {
        // GitHub adds values. A state we cannot read has not delivered a
        // verdict, so it counts as running rather than becoming a silent pass.
        //
        // This rung has no timeout, so an unrecognised *terminal* state renders
        // as "CI running" indefinitely: no later attempt is coming. That is a
        // known gap, watched by `scripts/check-states.sh`, not a property to
        // design around.
        assert_eq!(
            rule_requiring(&[REQUIRED]).and_then(|policy| decided(&policy, |pr| {
                pr.checks = reported(&[(REQUIRED, "SOMETHING_NEW")])?;
                Ok(())
            })),
            Ok(BlockedReason::CiRunning),
            "a state no variant names must not become a pass, and must not invent a blocker"
        );
    }

    #[test]
    fn a_pull_request_someone_was_asked_to_review_names_who() {
        assert_eq!(
            rule_requiring(&[REQUIRED])
                .and_then(|policy| decided(&policy, |pr| {
                    pr.requested_reviewers = vec![
                        Reviewer::User(Login::parse("hubot")?),
                        Reviewer::Bot(Login::parse("dependabot[bot]")?),
                    ];
                    Ok(())
                }))
                .map(waiting_for_review),
            Ok(Ok(vec![
                Some("hubot".to_owned()),
                Some("dependabot[bot]".to_owned())
            ])),
            "the accounts asked to look must survive to the reason, in the order given"
        );
    }

    #[test]
    fn a_request_addressed_to_a_team_is_still_somebody() {
        // With `requiresCodeOwnerReviews` on, a CODEOWNERS team request makes
        // the viewer an implicit reviewer who never appears by name. A team
        // request names no account, but somebody was asked, and rendering this
        // as "nobody assigned" would tell a team to go and find a reviewer they
        // already have. Whether the viewer is *in* that team needs membership
        // data this crate does not have.
        assert_eq!(
            rule_requiring(&[REQUIRED])
                .and_then(|policy| decided(&policy, |pr| {
                    pr.requested_reviewers = vec![Reviewer::Team(TeamSlug::new("ledger-owners"))];
                    Ok(())
                }))
                .map(waiting_for_review),
            Ok(Ok(vec![Some("ledger-owners".to_owned())])),
            "a team request is a request, so it cannot fall through to nobody-assigned"
        );
    }

    #[test]
    fn a_request_from_a_kind_this_crate_cannot_name_is_still_somebody() {
        // Reading `.login // .slug` and dropping a reviewer that has neither
        // renders the pull request as unassigned, which is a lie in the
        // direction that costs most. A request that exists must
        // not shorten the list into a different rung.
        assert_eq!(
            rule_requiring(&[REQUIRED])
                .and_then(|policy| decided(&policy, |pr| {
                    pr.requested_reviewers = vec![Reviewer::unknown("Organization")];
                    Ok(())
                }))
                .map(waiting_for_review),
            Ok(Ok(vec![None])),
            "a reviewer with no name is still a reviewer, and the rung must say so"
        );
    }

    #[test]
    fn a_reason_cannot_say_review_is_awaited_from_nobody() {
        assert_eq!(
            NonEmpty::new(Vec::<Reviewer>::new()),
            None,
            "an empty list of reviewers is nobody-assigned, not a pending review"
        );
    }

    /// One pull request wearing every rung's condition at once.
    ///
    /// Deliberately not a realistic payload — `reviewDecision` says approved
    /// while a reviewer still has changes outstanding, which GitHub would
    /// normally not report together. Precedence is only observable where the
    /// rungs overlap, and a pull request that satisfies one condition can only
    /// ever demonstrate one rung.
    fn every_condition_at_once() -> Result<PrFacts, InvalidValue> {
        let mut pr = open_pull_request()?;
        pr.draft_state = DraftState::Draft;
        pr.changes_requested_by = vec![Login::parse("hubot")?];
        pr.checks = reported(&[(REQUIRED, "FAILURE"), (REQUIRED_TITLE, "IN_PROGRESS")])?;
        pr.unresolved_threads = 2;
        pr.review_decision = Some(ReviewDecision::Approved);
        pr.mergeable = MergeableState::Conflicting;
        // `mergeable` and `mergeStateStatus` arrive from separate fields, so a
        // fixture can hold a conflict and an out-of-date branch at once even
        // though GitHub would report `DIRTY` for the first.
        pr.merge_state = MergeState::Behind;
        pr.requested_reviewers = vec![Reviewer::Team(TeamSlug::new("ledger-owners"))];
        Ok(pr)
    }

    /// One step of a ladder walk: a change to a pull request that should move
    /// it down exactly one rung.
    type Peel = fn(&mut PrFacts) -> Result<(), InvalidValue>;

    /// The rung names both ladder walks reached, or the parse failure that
    /// stopped one before it could.
    type Walks = Result<(Vec<&'static str>, Vec<&'static str>), InvalidValue>;

    /// Walks `start` down the ladder, applying one peel at a time and naming
    /// the rung reached after each.
    fn walk(
        policy: &PolicyRules,
        start: PrFacts,
        peels: &[Peel],
    ) -> Result<Vec<&'static str>, InvalidValue> {
        let mut pr = start;
        let mut reached = vec![rung(&BlockedReason::decide(&pr, policy))];
        for peel in peels {
            peel(&mut pr)?;
            reached.push(rung(&BlockedReason::decide(&pr, policy)));
        }
        Ok(reached)
    }

    /// The rule that makes every gated rung reachable: it names a required
    /// check, demands resolved threads, and demands a current branch.
    fn the_strictest_rule() -> Result<PolicyRules, InvalidValue> {
        rule_requiring(&[REQUIRED])
            .map(requiring_resolution)
            .map(demanding_a_current_branch)
    }

    #[test]
    fn the_ladder_answers_the_highest_rung_that_matches() {
        // The whole contract in two assertions: start from a pull request that
        // matches every rung, then take the conditions away one at a time and
        // watch the answer walk down. Any two rungs swapped shows up as a diff
        // of the entire ladder rather than as one opaque inequality.
        //
        // Two walks, because the two conflict rungs are one decision branching
        // on approval — a single monotonic peel cannot reach both without
        // toggling the approval back on, which is not a peel.
        const APPROVED: [Peel; 8] = [
            |pr| {
                pr.draft_state = DraftState::ReadyForReview;
                Ok(())
            },
            |pr| {
                pr.changes_requested_by = Vec::new();
                Ok(())
            },
            |pr| {
                pr.checks = reported(&[(REQUIRED, "SUCCESS"), (REQUIRED_TITLE, "IN_PROGRESS")])?;
                Ok(())
            },
            |pr| {
                pr.unresolved_threads = 0;
                Ok(())
            },
            |pr| {
                pr.mergeable = MergeableState::Mergeable;
                Ok(())
            },
            |pr| {
                pr.merge_state = MergeState::Blocked;
                Ok(())
            },
            |pr| {
                pr.checks = reported(&[(REQUIRED, "SUCCESS"), (REQUIRED_TITLE, "SUCCESS")])?;
                Ok(())
            },
            // Approved and green, and GitHub has not confirmed the merge is
            // clean. This must not read as ready to merge.
            |pr| {
                pr.mergeable = MergeableState::Computing;
                Ok(())
            },
        ];

        // No `behind-base` step: that rung is inside the approval, so an
        // unapproved branch that is behind reports the reviewer it is waiting
        // on instead. `a_branch_behind_its_base_is_not_a_reason_before_anyone
        // _has_approved_it` is what pins that.
        const UNAPPROVED: [Peel; 4] = [
            |pr| {
                pr.mergeable = MergeableState::Mergeable;
                Ok(())
            },
            |pr| {
                pr.checks = reported(&[(REQUIRED, "SUCCESS"), (REQUIRED_TITLE, "SUCCESS")])?;
                pr.mergeable = MergeableState::Computing;
                Ok(())
            },
            |pr| {
                pr.mergeable = MergeableState::Mergeable;
                Ok(())
            },
            |pr| {
                pr.requested_reviewers = Vec::new();
                Ok(())
            },
        ];

        let unapproved_start = || -> Result<PrFacts, InvalidValue> {
            let mut pr = every_condition_at_once()?;
            pr.draft_state = DraftState::ReadyForReview;
            pr.changes_requested_by = Vec::new();
            pr.unresolved_threads = 0;
            pr.review_decision = Some(ReviewDecision::ReviewRequired);
            pr.checks = reported(&[(REQUIRED, "SUCCESS"), (REQUIRED_TITLE, "IN_PROGRESS")])?;
            Ok(pr)
        };

        let walked = || -> Walks {
            let policy = the_strictest_rule()?;
            Ok((
                walk(&policy, every_condition_at_once()?, &APPROVED)?,
                walk(&policy, unapproved_start()?, &UNAPPROVED)?,
            ))
        };

        assert_eq!(
            walked(),
            Ok((
                vec![
                    "draft",
                    "changes-requested",
                    "required-check-failing",
                    "unresolved-threads",
                    "approved-but-conflicting",
                    "behind-base",
                    "approved-ci-running",
                    "ready-to-merge",
                    "mergeability-not-known",
                ],
                vec![
                    "conflicting",
                    "ci-running",
                    "mergeability-not-known",
                    "waiting-for-review",
                    "nobody-assigned",
                ],
            )),
            "taking one condition away at a time must walk the ladder in order"
        );
    }

    #[test]
    fn a_draft_outranks_even_a_failing_required_check() {
        // A draft is not asking for anything, so nothing else about it is a
        // reason to interrupt anyone. Reporting the red check instead would
        // nudge an author about work they have not finished writing.
        assert_eq!(
            rule_requiring(&[REQUIRED])
                .and_then(|policy| decided(&policy, |pr| {
                    pr.draft_state = DraftState::Draft;
                    pr.checks = reported(&[(REQUIRED, "FAILURE")])?;
                    Ok(())
                }))
                .as_ref()
                .map(rung),
            Ok("draft"),
            "the first rung wins over the third"
        );
    }

    #[test]
    fn changes_requested_outranks_a_failing_required_check() {
        // Both are true and only one is actionable. The check will be re-run by
        // the next push anyway; the review will not answer itself.
        assert_eq!(
            rule_requiring(&[REQUIRED])
                .and_then(|policy| decided(&policy, |pr| {
                    pr.changes_requested_by = vec![Login::parse("hubot")?];
                    pr.checks = reported(&[(REQUIRED, "FAILURE")])?;
                    Ok(())
                }))
                .as_ref()
                .map(rung),
            Ok("changes-requested"),
            "the second rung wins over the third"
        );
    }

    #[test]
    fn a_failing_required_check_outranks_an_unresolved_thread() {
        assert_eq!(
            rule_requiring(&[REQUIRED])
                .map(requiring_resolution)
                .and_then(|policy| decided(&policy, |pr| {
                    pr.checks = reported(&[(REQUIRED, "FAILURE")])?;
                    pr.unresolved_threads = 4;
                    Ok(())
                }))
                .as_ref()
                .map(rung),
            Ok("required-check-failing"),
            "the third rung wins over the fourth"
        );
    }

    #[test]
    fn an_unresolved_thread_outranks_an_approval() {
        // Approved and green, and the repository still will not merge it. The
        // approval is the tempting answer and the wrong one: reporting it as
        // ready to merge sends someone to press a button that is disabled.
        assert_eq!(
            rule_requiring(&[REQUIRED])
                .map(requiring_resolution)
                .and_then(|policy| decided(&policy, |pr| {
                    pr.unresolved_threads = 1;
                    pr.review_decision = Some(ReviewDecision::Approved);
                    pr.checks = reported(&[(REQUIRED, "SUCCESS")])?;
                    Ok(())
                }))
                .as_ref()
                .map(rung),
            Ok("unresolved-threads"),
            "the fourth rung wins over the seventh"
        );
    }

    #[test]
    fn approved_and_conflicting_outranks_ready_to_merge() {
        assert_eq!(
            rule_requiring(&[REQUIRED])
                .and_then(|policy| decided(&policy, |pr| {
                    pr.review_decision = Some(ReviewDecision::Approved);
                    pr.mergeable = MergeableState::Conflicting;
                    pr.merge_state = MergeState::Dirty;
                    pr.checks = reported(&[(REQUIRED, "SUCCESS")])?;
                    Ok(())
                }))
                .as_ref()
                .map(rung),
            Ok("approved-but-conflicting"),
            "the fifth rung wins over the seventh"
        );
    }

    #[test]
    fn ci_running_outranks_a_review_that_has_been_asked_for() {
        assert_eq!(
            rule_requiring(&[REQUIRED])
                .and_then(|policy| decided(&policy, |pr| {
                    pr.checks = reported(&[(ADVISORY, "IN_PROGRESS")])?;
                    pr.requested_reviewers = vec![Reviewer::User(Login::parse("hubot")?)];
                    Ok(())
                }))
                .as_ref()
                .map(rung),
            Ok("ci-running"),
            "the eighth rung wins over the ninth"
        );
    }

    #[test]
    fn ready_to_merge_requires_a_merge_github_has_confirmed_is_clean() {
        // `ReadyToMerge` must mean `MERGEABLE`, not merely "not CONFLICTING".
        // Both remaining states are answers GitHub has not given: `Computing`
        // is still being worked out in the background, and `Unknown` is a value
        // this crate could not read. Neither says the merge is clean, and
        // "go and press the button" is the expensive thing to say on a guess.
        for (label, mergeable) in [
            ("still being computed", MergeableState::Computing),
            (
                "a value no variant names",
                MergeableState::parse("SOMETHING_NEW"),
            ),
        ] {
            assert_eq!(
                rule_requiring(&[REQUIRED])
                    .and_then(|policy| decided(&policy, |pr| {
                        pr.review_decision = Some(ReviewDecision::Approved);
                        pr.mergeable = mergeable.clone();
                        pr.merge_state = MergeState::Undetermined;
                        pr.checks = reported(&[(REQUIRED, "SUCCESS")])?;
                        Ok(())
                    }))
                    .map(|reason| rung(&reason)),
                Ok("mergeability-not-known"),
                "mergeability {label} is not a clean merge, so it is not ready to merge"
            );
        }
    }

    #[test]
    fn mergeability_that_is_not_known_still_loses_to_a_build_that_is_running() {
        // Both are "wait", and CI is the longer and more informative wait:
        // mergeability resolves in seconds, a build does not.
        //
        // Both sides of the approval are asserted on purpose. The approved
        // block returns before the tail that orders these two, so an
        // approved-only version of this test passes whichever way round the
        // tail is written — it was exactly that, and a mutation swapping the
        // two rungs walked straight through it.
        for (label, decision, expected) in [
            (
                "approved",
                Some(ReviewDecision::Approved),
                "approved-ci-running",
            ),
            (
                "not yet approved",
                Some(ReviewDecision::ReviewRequired),
                "ci-running",
            ),
        ] {
            assert_eq!(
                rule_requiring(&[REQUIRED])
                    .and_then(|policy| decided(&policy, |pr| {
                        pr.review_decision = decision.clone();
                        pr.mergeable = MergeableState::Computing;
                        pr.merge_state = MergeState::Undetermined;
                        pr.checks = reported(&[(REQUIRED, "IN_PROGRESS")])?;
                        pr.requested_reviewers = vec![Reviewer::User(Login::parse("hubot")?)];
                        Ok(())
                    }))
                    .map(|reason| rung(&reason)),
                Ok(expected),
                "{label}: an unfinished build outranks an unfinished mergeability computation"
            );
        }
    }

    #[test]
    fn a_branch_behind_its_base_cannot_merge_where_the_rule_demands_it_be_current() {
        // `up_to_date_branch` is GitHub's `requiresStrictStatusChecks`. Where
        // the rule sets it, a branch merely behind its base — not in conflict —
        // cannot merge and GitHub disables the button. Reporting this one as
        // ready sends someone to press a button that does not work.
        assert_eq!(
            rule_requiring(&[REQUIRED])
                .map(demanding_a_current_branch)
                .and_then(|policy| decided(&policy, |pr| {
                    pr.review_decision = Some(ReviewDecision::Approved);
                    pr.mergeable = MergeableState::Mergeable;
                    pr.merge_state = MergeState::Behind;
                    pr.checks = reported(&[(REQUIRED, "SUCCESS")])?;
                    Ok(())
                }))
                .map(|reason| rung(&reason)),
            Ok("behind-base"),
            "approved and green is still not mergeable while the branch is out of date"
        );
    }

    #[test]
    fn a_branch_behind_its_base_is_not_a_reason_before_anyone_has_approved_it() {
        // The bottleneck here is the reviewer, not the author.
        //
        // Updating a branch changes not one line of what the author wrote, so
        // the reviewer reads the identical diff either way — unlike a conflict,
        // whose resolution rewrites it, which is why the two are not ranked
        // alike. And GitHub reports `BEHIND` only where strict status checks
        // are on, where a pull request goes behind every time the base moves:
        // ranking it above the review request would mask that request for most
        // pull requests most of the time on exactly the repositories that
        // enable the flag, and with stale-review dismissal it invites an
        // update-review-update loop.
        assert_eq!(
            rule_requiring(&[REQUIRED])
                .map(demanding_a_current_branch)
                .and_then(|policy| decided(&policy, |pr| {
                    pr.review_decision = Some(ReviewDecision::ReviewRequired);
                    pr.mergeable = MergeableState::Mergeable;
                    pr.merge_state = MergeState::Behind;
                    pr.checks = reported(&[(REQUIRED, "SUCCESS")])?;
                    pr.requested_reviewers = vec![Reviewer::User(Login::parse("hubot")?)];
                    Ok(())
                }))
                .map(|reason| rung(&reason)),
            Ok("waiting-for-review"),
            "being behind only matters once it is the last thing before the button"
        );
    }

    #[test]
    fn being_behind_is_skipped_entirely_where_the_rule_does_not_demand_a_current_branch() {
        // The same gating as unresolved threads. Without strict status checks
        // an out-of-date branch merges perfectly well, so naming it would send
        // an author to update a branch that was never in the way.
        assert_eq!(
            rule_requiring(&[REQUIRED])
                .and_then(|policy| decided(&policy, |pr| {
                    pr.review_decision = Some(ReviewDecision::Approved);
                    pr.mergeable = MergeableState::Mergeable;
                    pr.merge_state = MergeState::Behind;
                    pr.checks = reported(&[(REQUIRED, "SUCCESS")])?;
                    Ok(())
                }))
                .map(|reason| rung(&reason)),
            Ok("ready-to-merge"),
            "being behind is only a reason where the repository says it is"
        );
    }

    #[test]
    fn a_reported_check_reaches_the_decision_as_the_state_it_reported() {
        // Guards the fixture itself: every test about CI below depends on
        // `reported` producing checks that parse, and an empty rollup would
        // make several of them pass for the wrong reason.
        assert_eq!(
            reported(&[("Preview CI Gate", "FAILURE")]).map(|checks| checks
                .iter()
                .map(|check| (check.name().as_str().to_owned(), check.state().clone()))
                .collect::<Vec<_>>()),
            Ok(vec![("Preview CI Gate".to_owned(), CheckState::Failure)]),
            "the rollup fixture must produce the check it names"
        );
    }
}
