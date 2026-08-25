//! What a pull request's checks say, sorted by what each can do to the merge.

use chrono::{DateTime, Utc};

use crate::policy::PolicyRules;
use crate::types::{wire_value, CheckName};

/// The state a single check reports on a commit.
///
/// GitHub reports two kinds of check side by side in one rollup, and they use
/// different fields: a check run carries a `conclusion` once it finishes and a
/// `status` while it is still going, while a status context carries a `state`.
/// The three vocabularies overlap and no decision in this crate depends on
/// which of them a value came from — a failed status context and a failed check
/// run block a merge on exactly the same terms — so they fold into one type
/// here. Keeping them apart would push a join onto every consumer and buy
/// nothing back.
///
/// # An unrecognised state stays unrecognised
///
/// GitHub adds values. [`CheckState::Unknown`] holds the text of one this crate
/// has never seen, rather than mapping it onto the nearest known state, so a
/// new value shows up as something a human can read instead of quietly becoming
/// a pass. [`classify`] counts it as still running: a check whose meaning we
/// cannot read has not delivered a verdict, and calling it a failure would
/// invent a blocker while calling it a pass would hide one.
///
/// **That rule is safe only for a state still in flight, and nothing in this
/// crate can tell the difference.** An unrecognised *terminal* state renders as
/// "CI running" forever, because no later attempt is coming to correct it, and
/// no test here can catch it — the vocabulary a repository actually emits is
/// not knowable from inside this crate. `scripts/check-states.sh` is the only
/// detector: it reads the match arms below as its source of truth, reports what
/// a repository really produced, and exits non-zero on anything unrecognised.
/// Run it against every repository prtrack is pointed at. Naming `CANCELLED`
/// and `STALE` closed the two instances that had occurred, not the class.
///
/// The enum itself is *not* `#[non_exhaustive]`, so a match over it stays
/// exhaustive without a wildcard arm and a variant added here later breaks
/// every consumer that must decide what the new state means. Only the `Unknown`
/// variant is sealed, and only against construction:
///
/// ```
/// use prtrack_core::CheckState;
///
/// let state = CheckState::parse("SOMETHING_NEW");
/// assert!(matches!(state, CheckState::Unknown { .. }));
/// ```
///
/// Forging one outside this crate does not compile, so no caller can label a
/// state unknown that [`CheckState::parse`] would have recognised — which would
/// turn a failure into a check that merely looks unfinished. Only the middle
/// line differs from the block above:
///
/// ```compile_fail,E0639
/// use prtrack_core::CheckState;
///
/// let state = CheckState::Unknown { reported: "FAILURE".to_owned() };
/// assert!(matches!(state, CheckState::Unknown { .. }));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckState {
    /// `FAILURE` — the check ran and reported failure.
    Failure,
    /// `ERROR` — a status context could not produce a result.
    Error,
    /// `TIMED_OUT` — the check ran out of time.
    TimedOut,
    /// `STARTUP_FAILURE` — the check never got as far as running.
    StartupFailure,
    /// `ACTION_REQUIRED` — the check finished and is asking for a human.
    ///
    /// Failing rather than running: nothing further happens on its own, so the
    /// pull request is stuck until someone acts.
    ActionRequired,
    /// `CANCELLED` — the attempt was stopped before it could reach a verdict.
    ///
    /// Failing, not running. A cancelled attempt has not passed and will not
    /// resume on its own. Where a later attempt supersedes it this changes
    /// nothing, because [`classify`] keeps only the newest; where it *is* the
    /// newest, the check genuinely did not pass. Measured on the target
    /// repository this is the fourth most common state, at 45 occurrences, so
    /// misfiling it is not a rare-path mistake.
    Cancelled,
    /// `STALE` — GitHub discarded the result before it could be used.
    ///
    /// Failing for the same reason as [`CheckState::Cancelled`]: the attempt
    /// never reached success.
    Stale,

    /// `IN_PROGRESS` — the check is running now.
    InProgress,
    /// `QUEUED` — the check is waiting for a runner.
    Queued,
    /// `PENDING` — the check has been created but has not reported.
    Pending,
    /// `WAITING` — the check is held before it starts, as by a deployment gate.
    Waiting,
    /// `EXPECTED` — a status context the repository expects and has not
    /// received.
    Expected,

    /// `SUCCESS` — the check passed.
    Success,
    /// `SKIPPED` — the check decided it had nothing to do.
    Skipped,
    /// `NEUTRAL` — the check finished deliberately without a verdict.
    Neutral,

    /// A state no variant above names, kept as the text it arrived as.
    ///
    /// `#[non_exhaustive]` so that no other crate can build one. Reading it
    /// stays open, which is what lets a renderer show the value GitHub sent;
    /// forging one does not.
    #[non_exhaustive]
    Unknown {
        /// The state exactly as it was reported, trimmed.
        reported: String,
    },
}

impl CheckState {
    /// Reads a state from the text GitHub reported for a check.
    ///
    /// Never fails. An unrecognised value becomes [`CheckState::Unknown`]
    /// rather than an error, because a check this crate cannot interpret must
    /// still appear on the board: dropping it would understate what a pull
    /// request is waiting on, which is the one failure mode this crate exists
    /// to avoid.
    ///
    /// Surrounding whitespace is stripped, and ASCII case is folded. Unlike
    /// [`CheckName`], where case is meaning — GitHub matches a required context
    /// against a reported name literally — a state is an API enum whose values
    /// are unique whatever their case, so `FAILURE` and `failure` are the same
    /// state and reading them as two would report a failure as merely
    /// unfinished.
    ///
    /// # Examples
    ///
    /// ```
    /// use prtrack_core::CheckState;
    ///
    /// assert_eq!(CheckState::parse("  TIMED_OUT\n"), CheckState::TimedOut);
    /// assert_eq!(CheckState::parse("failure"), CheckState::Failure);
    /// assert!(matches!(CheckState::parse(""), CheckState::Unknown { .. }));
    /// ```
    ///
    /// [`CheckName`]: crate::CheckName
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        // Normalised once, in `types`, so this table and the ones in `facts`
        // cannot come to disagree about what the same value means. The arms
        // below stay a plain table of literals: `scripts/check-states.sh`
        // greps them as its source of truth for the recognised vocabulary.
        let (reported, key) = wire_value(raw);
        match key.as_str() {
            "FAILURE" => Self::Failure,
            "ERROR" => Self::Error,
            "TIMED_OUT" => Self::TimedOut,
            "STARTUP_FAILURE" => Self::StartupFailure,
            "ACTION_REQUIRED" => Self::ActionRequired,
            "CANCELLED" => Self::Cancelled,
            "STALE" => Self::Stale,
            "IN_PROGRESS" => Self::InProgress,
            "QUEUED" => Self::Queued,
            "PENDING" => Self::Pending,
            "WAITING" => Self::Waiting,
            "EXPECTED" => Self::Expected,
            "SUCCESS" => Self::Success,
            "SKIPPED" => Self::Skipped,
            "NEUTRAL" => Self::Neutral,
            _ => Self::Unknown {
                reported: reported.to_owned(),
            },
        }
    }
}

/// What a state means for the merge, before the policy has had its say.
///
/// Deliberately not public. Which of three buckets a check falls into is only
/// half an answer — a failing check may or may not block, and only the policy
/// knows which — so the useful thing to hand a caller is the whole partition
/// [`Classified`] holds, not a per-check label they would have to combine with
/// the policy themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// The check finished badly. Whether that blocks is the policy's call.
    Failing,
    /// The check has not finished, so there is no verdict to act on yet.
    Running,
    /// The check finished and holds nothing up.
    Passing,
}

impl CheckState {
    /// What this state means for the merge, before the policy has had its say.
    const fn outcome(&self) -> Outcome {
        match self {
            Self::Failure
            | Self::Error
            | Self::TimedOut
            | Self::StartupFailure
            | Self::ActionRequired
            | Self::Cancelled
            | Self::Stale => Outcome::Failing,
            Self::InProgress | Self::Queued | Self::Pending | Self::Waiting | Self::Expected => {
                Outcome::Running
            }
            Self::Success | Self::Skipped | Self::Neutral => Outcome::Passing,
            // A state this crate cannot read has not delivered a verdict.
            // Calling it a failure would invent a blocker; calling it a pass
            // would hide one.
            Self::Unknown { .. } => Outcome::Running,
        }
    }
}

/// One attempt at one check, and when it reported.
///
/// GitHub reports every *attempt*, not every check: a re-run leaves the run it
/// replaced in the rollup beside it, so one name arrives several times with
/// different verdicts. Measured on the target repository, 9 of 38 open pull
/// requests carry at least one duplicated name. Only the newest attempt is
/// current, so an instant is not decoration here — without it [`classify`]
/// cannot tell a superseded failure from a live one. One wrong verdict on a
/// *required* check is enough to justify the rule: `Validate PR Title` on
/// `#4346` went `FAILURE` then `SUCCESS`, and reading the failure as current
/// was a live false red.
///
/// `reported_at` is when *this attempt* reported: `startedAt` on a check run,
/// `createdAt` on a status context. It is optional because the first of those
/// is nullable — a check that never started has no instant, and inventing one
/// would be fabricating data. An attempt with no instant loses to any attempt
/// that has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    name: CheckName,
    state: CheckState,
    reported_at: Option<DateTime<Utc>>,
}

impl Check {
    /// Records one attempt at one check.
    ///
    /// # Examples
    ///
    /// ```
    /// use chrono::{DateTime, Utc};
    /// use prtrack_core::{Check, CheckName, CheckState};
    ///
    /// let started = DateTime::<Utc>::UNIX_EPOCH;
    /// let attempt = Check::new(
    ///     CheckName::parse("Preview CI Gate")?,
    ///     CheckState::parse("SUCCESS"),
    ///     Some(started),
    /// );
    /// assert_eq!(attempt.state(), &CheckState::Success);
    /// assert_eq!(attempt.reported_at(), Some(started));
    ///
    /// // A check run with a null `startedAt` has no instant, and none is
    /// // invented for it.
    /// let never_started = Check::new(
    ///     CheckName::parse("Preview CI Gate")?,
    ///     CheckState::parse("QUEUED"),
    ///     None,
    /// );
    /// assert_eq!(never_started.reported_at(), None);
    /// # Ok::<(), prtrack_core::InvalidValue>(())
    /// ```
    #[must_use]
    pub const fn new(
        name: CheckName,
        state: CheckState,
        reported_at: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            name,
            state,
            reported_at,
        }
    }

    /// The check's name, as the check reports it.
    #[must_use]
    pub const fn name(&self) -> &CheckName {
        &self.name
    }

    /// What this attempt reported.
    #[must_use]
    pub const fn state(&self) -> &CheckState {
        &self.state
    }

    /// When this attempt reported, if it ever started.
    #[must_use]
    pub const fn reported_at(&self) -> Option<DateTime<Utc>> {
        self.reported_at
    }
}

/// A pull request's checks, split by what each can do to the merge.
///
/// Four disjoint lists rather than four counts, because the counts are
/// `.len()` and the names are what a board renders: "one required check
/// failing" is a number, but "`Preview CI Gate` failing" is the sentence that
/// tells someone what to go and fix.
///
/// The fields are private and [`classify`] is the only thing that fills them,
/// so a check cannot appear in two lists, and a list cannot be assembled from
/// checks that were never classified against a policy.
///
/// # What the lists mean
///
/// - **blocking** — failing, and the policy says a failure of this check holds
///   the merge back.
/// - **advisory** — failing, and the policy says it does not. Most checks on a
///   pull request are these.
/// - **running** — no verdict yet, including any state this crate does not
///   recognise. See [`CheckState`].
/// - **passing** — finished, holding nothing up.
///
/// A check name that reported more than once appears once, at its newest
/// attempt, so `.len()` counts checks rather than attempts. See [`classify`].
///
/// # An empty rollup is not a green one
///
/// A pull request whose checks all passed has a non-empty `passing` list. A
/// pull request that reported no checks at all has [`Classified::total`] of
/// zero. The two must stay distinguishable: rendering "CI never ran" as "CI is
/// green" is exactly the stale cell wearing a fresh face that this crate exists
/// to prevent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classified {
    blocking: Vec<Check>,
    advisory: Vec<Check>,
    running: Vec<Check>,
    passing: Vec<Check>,
}

impl Classified {
    /// The failing checks the policy says can hold the merge back.
    #[must_use]
    pub fn blocking(&self) -> &[Check] {
        &self.blocking
    }

    /// The failing checks the policy says cannot hold the merge back.
    #[must_use]
    pub fn advisory(&self) -> &[Check] {
        &self.advisory
    }

    /// The checks that have not delivered a verdict yet, including any whose
    /// state this crate does not recognise.
    #[must_use]
    pub fn running(&self) -> &[Check] {
        &self.running
    }

    /// The checks that finished without holding anything up.
    #[must_use]
    pub fn passing(&self) -> &[Check] {
        &self.passing
    }

    /// How many distinct checks reported.
    ///
    /// Zero means the rollup was empty — nothing has reported — which is not
    /// the same as everything having passed. A name that reported twice counts
    /// once.
    #[must_use]
    pub const fn total(&self) -> usize {
        self.blocking.len() + self.advisory.len() + self.running.len() + self.passing.len()
    }
}

/// Reduces a name's repeated attempts to the newest one, which is the only
/// current one.
///
/// The rollup carries every attempt, so counting them all overstates what a
/// pull request is waiting on — and, worse, lets a superseded verdict speak for
/// a check that has since been re-run. Recency is the whole rule: **no
/// property of the states themselves is consulted.** Merging by outcome
/// instead gives wrong answers in both directions, verified against the live
/// repository — a required check that went `FAILURE` then `SUCCESS` read as
/// failing, and a superseded `CANCELLED` masked the real verdict beside it.
///
/// An attempt with no instant loses to any attempt that has one, which falls
/// out of `Option`'s own ordering rather than being special-cased.
///
/// Ties are real, and the reason is in this crate's own types: `reported_at`
/// is an `Option`, `startedAt` is nullable, and two attempts that never
/// started therefore carry the same absent instant and tie exactly. Two dated
/// attempts can also share a second. Separately, GitHub's timestamps are not
/// always internally consistent — `#4324` carries a `startedAt` of `21:24:07`
/// beside a `completedAt` of `21:24:06` — which is non-monotonicity within one
/// run rather than a tie between two, and is recorded here as its own
/// observation. The tie-break is **the attempt that arrived later wins**,
/// which is the only rule that cannot invent a winner: the newest attempt is
/// the current one. A different rule would decide a case that is ambiguous
/// rather than
/// wrong.
///
/// The order names first appeared in is kept, so a bucket reads in the order
/// the rollup gave.
fn merged_by_name(checks: &[Check]) -> Vec<Check> {
    let mut merged: Vec<Check> = Vec::with_capacity(checks.len());

    for attempt in checks {
        match merged.iter_mut().find(|kept| kept.name == attempt.name) {
            // `>=` rather than `>` *is* the tie-break: on an equal instant the
            // later arrival replaces the one standing. Narrowing this to `>`
            // would silently reverse the answer whenever two attempts share a
            // timestamp.
            Some(standing) if attempt.reported_at >= standing.reported_at => {
                *standing = attempt.clone();
            }
            Some(_superseded) => {}
            None => merged.push(attempt.clone()),
        }
    }

    merged
}

/// Sorts a pull request's checks into what blocks the merge, what is merely
/// advisory, what is still running, and what passed.
///
/// `checks` is the rollup as reported: every *attempt*, not every check. Check
/// runs and status contexts are not distinguished, because nothing about the
/// verdict depends on which of them reported — see [`CheckState`].
///
/// # Only the newest attempt counts
///
/// One name reports many times, and a re-run does not remove the run it
/// replaced. Each name is therefore reduced to its newest attempt before
/// anything is classified, so a failure that has since been re-run green does
/// not report as a blocker, and a superseded cancellation does not mask the
/// verdict that followed it. Both of those were live wrong answers before this
/// rule existed. Ties go to the attempt that arrived later; an attempt with no
/// instant loses to one that has it.
///
/// # There is no `governed` flag
///
/// Whether a rule covers this pull request at all is the caller's question to
/// answer, and the answer is *which policy they pass*. A pull request no rule
/// governs — a stacked one aimed at another feature branch — is classified
/// against [`PolicyRules::unprotected`], whose
/// [`blocks_merge_when_failing`](PolicyRules::blocks_merge_when_failing) already
/// treats every failure as blocking. That keeps the fallback in one place
/// instead of repeating it as a boolean parameter that a caller could get
/// backwards.
///
/// # Examples
///
/// ```
/// use prtrack_core::{
///     classify, BlockingChecks, Check, CheckName, CheckState, PolicyRules, Requirement,
/// };
///
/// let gate = CheckName::parse("Preview CI Gate").unwrap();
/// let coverage = CheckName::parse("codecov/patch").unwrap();
///
/// let rule = PolicyRules {
///     blocking_checks: BlockingChecks::from_rule(Requirement::Required, vec![gate.clone()]),
///     ..PolicyRules::unprotected()
/// };
///
/// let reported = vec![
///     Check::new(gate, CheckState::parse("FAILURE"), None),
///     Check::new(coverage, CheckState::parse("FAILURE"), None),
/// ];
///
/// let classified = classify(&reported, &rule);
/// assert_eq!(classified.blocking().len(), 1);
/// assert_eq!(classified.advisory().len(), 1);
///
/// // With no rule to read, the same two failures are both blocking.
/// let unreadable = classify(&reported, &PolicyRules::unprotected());
/// assert_eq!(unreadable.blocking().len(), 2);
/// assert!(unreadable.advisory().is_empty());
/// ```
#[must_use]
pub fn classify(checks: &[Check], policy: &PolicyRules) -> Classified {
    let mut classified = Classified {
        blocking: Vec::new(),
        advisory: Vec::new(),
        running: Vec::new(),
        passing: Vec::new(),
    };

    for check in merged_by_name(checks) {
        let bucket = match check.state.outcome() {
            Outcome::Failing => {
                if policy.blocks_merge_when_failing(&check.name) {
                    &mut classified.blocking
                } else {
                    &mut classified.advisory
                }
            }
            Outcome::Running => &mut classified.running,
            Outcome::Passing => &mut classified.passing,
        };
        bucket.push(check);
    }

    classified
}

#[cfg(test)]
mod tests {
    use super::{classify, Check, CheckState, Classified};
    use crate::policy::{BlockingChecks, PolicyRules, Requirement};
    use crate::types::CheckName;
    use chrono::{DateTime, TimeDelta, Utc};

    /// A check the rule below names, so failing it holds the merge back.
    const REQUIRED: &str = "Preview CI Gate";

    /// Another check the rule names. On the live repository this is the one
    /// that reported `FAILURE` and then passed on re-run.
    const REQUIRED_RE_RUN: &str = "Validate PR Title";

    /// A check no rule names. It runs on every pull request and the merge
    /// button ignores it however red it goes.
    const ADVISORY: &str = "codecov/patch";

    /// Every state that means the check finished badly.
    ///
    /// `CANCELLED` and `STALE` belong here rather than beside the running
    /// states: both mean the attempt stopped without reaching success, and
    /// nothing further happens on its own. `CANCELLED` is the fourth most
    /// common state on the target repository.
    const FAILING: [&str; 7] = [
        "FAILURE",
        "ERROR",
        "TIMED_OUT",
        "STARTUP_FAILURE",
        "ACTION_REQUIRED",
        "CANCELLED",
        "STALE",
    ];

    /// Every state that means the check has not delivered a verdict yet.
    const RUNNING: [&str; 5] = ["IN_PROGRESS", "QUEUED", "PENDING", "WAITING", "EXPECTED"];

    /// Every state that means the check finished without holding anything up.
    const PASSING: [&str; 3] = ["SUCCESS", "SKIPPED", "NEUTRAL"];

    /// An instant `minutes` from an arbitrary base.
    ///
    /// No test cares what the wall-clock value is, only which of two attempts
    /// is the later one, so the base is meaningless on purpose.
    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::<Utc>::UNIX_EPOCH + TimeDelta::minutes(minutes)
    }

    /// Builds the attempts a pull request reported, where `None` for the
    /// instant is an attempt that never started — a check run with a null
    /// `startedAt`.
    ///
    /// Fails loudly rather than silently dropping a row: a fixture built from a
    /// name that did not parse would leave the assertions below asserting
    /// nothing. Every other fixture helper funnels through here, so that guard
    /// covers all of them.
    fn dated(rows: &[(&str, &str, Option<i64>)]) -> Vec<Check> {
        let parsed: Vec<Check> = rows
            .iter()
            .filter_map(|(name, state, minutes)| {
                CheckName::parse(name)
                    .ok()
                    .map(|name| Check::new(name, CheckState::parse(state), minutes.map(at)))
            })
            .collect();
        assert_eq!(
            parsed.len(),
            rows.len(),
            "every fixture check name must parse or this test proves nothing: {rows:?}"
        );
        parsed
    }

    /// Attempts stamped `minutes` from the base, for the tests about recency.
    fn attempts(rows: &[(&str, &str, i64)]) -> Vec<Check> {
        let dated_rows: Vec<(&str, &str, Option<i64>)> = rows
            .iter()
            .map(|(name, state, minutes)| (*name, *state, Some(*minutes)))
            .collect();
        dated(&dated_rows)
    }

    /// The same, for the tests where no name reports twice and the instant is
    /// therefore irrelevant. Every attempt is stamped at the same instant, so
    /// if one of these fixtures ever did repeat a name it would exercise the
    /// tie-break rather than recency — which is why the merge tests all use
    /// [`attempts`] instead.
    fn checks(rows: &[(&str, &str)]) -> Vec<Check> {
        let stamped: Vec<(&str, &str, i64)> = rows
            .iter()
            .map(|(name, state)| (*name, *state, 0))
            .collect();
        attempts(&stamped)
    }

    /// A rule that names exactly `required` and demands nothing else.
    fn rule_requiring(required: &[&str]) -> PolicyRules {
        let contexts: Vec<CheckName> = required
            .iter()
            .filter_map(|name| CheckName::parse(name).ok())
            .collect();
        assert_eq!(
            contexts.len(),
            required.len(),
            "every required context must parse or the rule under test is not the one described: {required:?}"
        );
        PolicyRules {
            blocking_checks: BlockingChecks::from_rule(Requirement::Required, contexts),
            ..PolicyRules::unprotected()
        }
    }

    /// The four buckets as labelled name lists, so one assertion pins the whole
    /// partition. A check that leaked into a neighbouring bucket, or was
    /// counted in two, fails the test that owns it instead of hiding behind an
    /// assertion that only looked at one list.
    fn buckets(classified: &Classified) -> [(&'static str, Vec<&str>); 4] {
        fn names(checks: &[Check]) -> Vec<&str> {
            checks.iter().map(|check| check.name().as_str()).collect()
        }
        [
            ("blocking", names(classified.blocking())),
            ("advisory", names(classified.advisory())),
            ("running", names(classified.running())),
            ("passing", names(classified.passing())),
        ]
    }

    /// The buckets that hold anything, for asserting where one check landed
    /// without spelling out three empty lists.
    fn occupied(classified: &Classified) -> Vec<(&'static str, Vec<&str>)> {
        buckets(classified)
            .into_iter()
            .filter(|(_, names)| !names.is_empty())
            .collect()
    }

    #[test]
    fn a_failing_check_the_rule_names_blocks_the_merge() {
        let policy = rule_requiring(&[REQUIRED]);
        for state in FAILING {
            let classified = classify(&checks(&[(REQUIRED, state)]), &policy);
            assert_eq!(
                occupied(&classified),
                vec![("blocking", vec![REQUIRED])],
                "{REQUIRED:?} reporting {state:?} is on the required list, so it must block"
            );
        }
    }

    #[test]
    fn a_failing_check_the_rule_does_not_name_is_advisory() {
        let policy = rule_requiring(&[REQUIRED]);
        for state in FAILING {
            let classified = classify(&checks(&[(ADVISORY, state)]), &policy);
            assert_eq!(
                occupied(&classified),
                vec![("advisory", vec![ADVISORY])],
                "{ADVISORY:?} reporting {state:?} is on no required list, so the merge button ignores it"
            );
        }
    }

    #[test]
    fn under_an_unprotected_branch_every_failing_check_blocks() {
        // The stacked-pull-request case: no rule covers the base branch, so the
        // policy told us nothing and nothing can be called harmless. There is
        // no `governed` flag anywhere in this crate; the caller says "no rule
        // applies" by passing this policy.
        let policy = PolicyRules::unprotected();
        for state in FAILING {
            let classified = classify(&checks(&[(REQUIRED, state), (ADVISORY, state)]), &policy);
            assert_eq!(
                occupied(&classified),
                vec![("blocking", vec![REQUIRED, ADVISORY])],
                "with no rule to read, {state:?} must block whatever the check is called"
            );
        }
    }

    #[test]
    fn a_running_check_has_not_delivered_a_verdict_either_way() {
        let policy = rule_requiring(&[REQUIRED]);
        for state in RUNNING {
            let classified = classify(&checks(&[(REQUIRED, state), (ADVISORY, state)]), &policy);
            assert_eq!(
                occupied(&classified),
                vec![("running", vec![REQUIRED, ADVISORY])],
                "{state:?} means the check has not finished, required or not"
            );
        }
    }

    #[test]
    fn a_check_that_finished_without_complaint_holds_nothing_up() {
        let policy = rule_requiring(&[REQUIRED]);
        for state in PASSING {
            let classified = classify(&checks(&[(REQUIRED, state), (ADVISORY, state)]), &policy);
            assert_eq!(
                occupied(&classified),
                vec![("passing", vec![REQUIRED, ADVISORY])],
                "{state:?} holds nothing up, required or not"
            );
        }
    }

    #[test]
    fn an_unknown_state_is_counted_as_running_and_keeps_the_text_it_arrived_as() {
        // GitHub adds values. A state we cannot read has not delivered a
        // verdict, so it counts as running — and it must stay legible, because
        // the alternative is a check that silently became a pass.
        let policy = rule_requiring(&[REQUIRED]);
        let classified = classify(&checks(&[(REQUIRED, "SOMETHING_NEW")]), &policy);

        assert_eq!(
            occupied(&classified),
            vec![("running", vec![REQUIRED])],
            "a state no variant names must be counted as still running"
        );
        assert_eq!(
            classified
                .running()
                .iter()
                .map(Check::state)
                .collect::<Vec<_>>(),
            vec![&CheckState::Unknown {
                reported: "SOMETHING_NEW".to_owned()
            }],
            "the reported text must survive classification so a human can read it"
        );
    }

    #[test]
    fn a_status_context_and_a_check_run_failing_together_are_both_counted() {
        // `ERROR` only ever comes from a status context and `STARTUP_FAILURE`
        // only from a check run, so this pair cannot be produced by one kind
        // alone. Both must survive: the two arrive from different fields of the
        // rollup and an implementation that read only one would lose half the
        // failures.
        let policy = PolicyRules::unprotected();
        let classified = classify(
            &checks(&[
                ("ci/circleci: build", "ERROR"),
                (REQUIRED, "STARTUP_FAILURE"),
            ]),
            &policy,
        );
        assert_eq!(
            occupied(&classified),
            vec![("blocking", vec!["ci/circleci: build", REQUIRED])],
            "a failing status context and a failing check run are two failures, not one"
        );
    }

    #[test]
    fn a_required_check_that_passed_on_re_run_does_not_block() {
        // Live, `Example-Org/example-repo#4346`: `Validate PR Title` reported
        // FAILURE and then SUCCESS three times. Reading the failure as current
        // reports a *required* check as failing when it passed — a false red on
        // the one kind of check that can hold a merge. Counting every
        // historical attempt as current is how a re-run that passed still
        // reads as a failure.
        let policy = rule_requiring(&[REQUIRED_RE_RUN]);
        let classified = classify(
            &attempts(&[
                (REQUIRED_RE_RUN, "FAILURE", 0),
                (REQUIRED_RE_RUN, "SUCCESS", 1),
                (REQUIRED_RE_RUN, "SUCCESS", 2),
                (REQUIRED_RE_RUN, "SUCCESS", 3),
            ]),
            &policy,
        );
        assert_eq!(
            occupied(&classified),
            vec![("passing", vec![REQUIRED_RE_RUN])],
            "the newest attempt passed, so nothing here holds the merge back"
        );
    }

    #[test]
    fn a_check_that_passed_and_then_failed_is_failing() {
        // The mirror of the case above, and the reason recency cannot be
        // approximated by "prefer the happier verdict": the newest attempt is
        // the only current one, whichever direction it went.
        let policy = rule_requiring(&[REQUIRED_RE_RUN]);
        let classified = classify(
            &attempts(&[
                (REQUIRED_RE_RUN, "SUCCESS", 0),
                (REQUIRED_RE_RUN, "FAILURE", 1),
            ]),
            &policy,
        );
        assert_eq!(
            occupied(&classified),
            vec![("blocking", vec![REQUIRED_RE_RUN])],
            "the newest attempt failed, so an earlier pass does not rescue it"
        );
    }

    #[test]
    fn the_newest_attempt_wins_however_the_rollup_ordered_them() {
        // Recency must be read from the instant, not from arrival order.
        // Nothing promises the rollup is chronological, and these two fixtures
        // are the same four attempts in different orders.
        let policy = rule_requiring(&[REQUIRED_RE_RUN]);
        let orderings: [[(&str, &str, i64); 4]; 3] = [
            [
                (REQUIRED_RE_RUN, "FAILURE", 0),
                (REQUIRED_RE_RUN, "SUCCESS", 3),
                (REQUIRED_RE_RUN, "FAILURE", 1),
                (REQUIRED_RE_RUN, "FAILURE", 2),
            ],
            [
                (REQUIRED_RE_RUN, "SUCCESS", 3),
                (REQUIRED_RE_RUN, "FAILURE", 2),
                (REQUIRED_RE_RUN, "FAILURE", 1),
                (REQUIRED_RE_RUN, "FAILURE", 0),
            ],
            [
                (REQUIRED_RE_RUN, "FAILURE", 2),
                (REQUIRED_RE_RUN, "FAILURE", 0),
                (REQUIRED_RE_RUN, "SUCCESS", 3),
                (REQUIRED_RE_RUN, "FAILURE", 1),
            ],
        ];
        for ordering in orderings {
            let classified = classify(&attempts(&ordering), &policy);
            assert_eq!(
                occupied(&classified),
                vec![("passing", vec![REQUIRED_RE_RUN])],
                "the attempt at minute 3 is the newest wherever it appears: {ordering:?}"
            );
        }
    }

    #[test]
    fn a_cancelled_attempt_a_later_one_replaced_does_not_hold_the_merge() {
        // Live, `#4155`: `Preview CI Gate` CANCELLED at 07:21, SUCCESS at
        // 08:23. Treating the cancellation as current masks the real verdict.
        let policy = rule_requiring(&[REQUIRED]);
        let classified = classify(
            &attempts(&[(REQUIRED, "CANCELLED", 0), (REQUIRED, "SUCCESS", 62)]),
            &policy,
        );
        assert_eq!(
            occupied(&classified),
            vec![("passing", vec![REQUIRED])],
            "a cancellation the next attempt superseded is not the current word"
        );
    }

    #[test]
    fn a_cancelled_attempt_that_is_the_newest_word_holds_the_merge() {
        // Live, `#4324`: CANCELLED at 21:24 then FAILURE at 21:26. And a
        // cancellation with nothing after it is itself a check that did not
        // pass — not one still in flight, because nothing further will happen
        // on its own.
        let policy = rule_requiring(&[REQUIRED]);
        let superseded_by_a_failure = classify(
            &attempts(&[(REQUIRED, "CANCELLED", 0), (REQUIRED, "FAILURE", 2)]),
            &policy,
        );
        assert_eq!(
            occupied(&superseded_by_a_failure),
            vec![("blocking", vec![REQUIRED])],
            "the newest attempt failed outright"
        );

        let cancelled_and_left = classify(&attempts(&[(REQUIRED, "CANCELLED", 0)]), &policy);
        assert_eq!(
            occupied(&cancelled_and_left),
            vec![("blocking", vec![REQUIRED])],
            "a required check that was cancelled and never re-run has not passed"
        );
    }

    #[test]
    fn attempts_carrying_the_same_instant_are_settled_by_the_order_they_arrived() {
        // Ties are real because `reported_at` is an `Option` and `startedAt`
        // is nullable: two attempts that never started share the same absent
        // instant exactly, and two dated ones can share a second. The last
        // attempt the rollup listed wins.
        //
        // (`#4324` carrying `startedAt 21:24:07` beside `completedAt 21:24:06`
        // is a separate observation — timestamps that disagree within one run,
        // not two attempts tying.)
        //
        // Ties are decided by arrival order, so
        // choosing a different rule here would fail acceptance on a case that
        // is ambiguous rather than wrong.
        let policy = rule_requiring(&[REQUIRED]);
        for (first, second, expected) in [
            ("FAILURE", "SUCCESS", "passing"),
            ("SUCCESS", "FAILURE", "blocking"),
            ("CANCELLED", "IN_PROGRESS", "running"),
            ("IN_PROGRESS", "CANCELLED", "blocking"),
        ] {
            let classified = classify(
                &attempts(&[(REQUIRED, first, 7), (REQUIRED, second, 7)]),
                &policy,
            );
            assert_eq!(
                occupied(&classified),
                vec![(expected, vec![REQUIRED])],
                "{second:?} arrived after {first:?} at the same instant, so it is the word that stands"
            );
        }
    }

    #[test]
    fn an_attempt_that_never_started_loses_to_one_that_did() {
        // `startedAt` is nullable on a check run, so an attempt can arrive with
        // no instant at all. It must not outrank a real report — and when
        // nothing has an instant, the tie-break above applies.
        let policy = rule_requiring(&[REQUIRED]);

        let undated_first = classify(
            &dated(&[(REQUIRED, "FAILURE", None), (REQUIRED, "SUCCESS", Some(0))]),
            &policy,
        );
        assert_eq!(
            occupied(&undated_first),
            vec![("passing", vec![REQUIRED])],
            "a dated attempt is newer than one that never started"
        );

        let undated_last = classify(
            &dated(&[(REQUIRED, "SUCCESS", Some(0)), (REQUIRED, "FAILURE", None)]),
            &policy,
        );
        assert_eq!(
            occupied(&undated_last),
            vec![("passing", vec![REQUIRED])],
            "arriving later does not make an undated attempt newer"
        );

        let neither_dated = classify(
            &dated(&[(REQUIRED, "SUCCESS", None), (REQUIRED, "FAILURE", None)]),
            &policy,
        );
        assert_eq!(
            occupied(&neither_dated),
            vec![("blocking", vec![REQUIRED])],
            "with no instant to compare, the last attempt listed stands"
        );
    }

    #[test]
    fn a_re_run_in_flight_supersedes_the_failure_it_repeats() {
        // The original packet case, now answered by recency rather than by
        // preferring the unfinished report: a re-run started *after* a failure
        // means the pull request is waiting, not blocked. Reverse the instants
        // and the failure is the current word.
        let policy = rule_requiring(&[REQUIRED]);

        let re_run_is_newer = classify(
            &attempts(&[(REQUIRED, "FAILURE", 0), (REQUIRED, "IN_PROGRESS", 1)]),
            &policy,
        );
        assert_eq!(
            occupied(&re_run_is_newer),
            vec![("running", vec![REQUIRED])],
            "one check, and its newest attempt is still going"
        );

        let failure_is_newer = classify(
            &attempts(&[(REQUIRED, "IN_PROGRESS", 0), (REQUIRED, "FAILURE", 1)]),
            &policy,
        );
        assert_eq!(
            occupied(&failure_is_newer),
            vec![("blocking", vec![REQUIRED])],
            "a failure recorded after a run started is the current word"
        );
    }

    #[test]
    fn checks_keep_the_order_they_were_reported_in() {
        let policy = PolicyRules::unprotected();
        let classified = classify(
            &checks(&[
                ("Validate PR Title", "FAILURE"),
                (ADVISORY, "SUCCESS"),
                (REQUIRED, "FAILURE"),
            ]),
            &policy,
        );
        assert_eq!(
            occupied(&classified),
            vec![
                ("blocking", vec!["Validate PR Title", REQUIRED]),
                ("passing", vec![ADVISORY]),
            ],
            "a bucket must list its checks in the order the rollup gave them"
        );
    }

    #[test]
    fn a_pull_request_with_no_checks_is_not_a_pull_request_whose_checks_all_passed() {
        // An empty rollup means CI never reported. Rendering that as "all
        // green" is the stale-cell-with-a-fresh-face failure this crate exists
        // to avoid, so the two cases must be distinguishable.
        let policy = rule_requiring(&[REQUIRED]);

        let nothing_reported = classify(&[], &policy);
        assert_eq!(
            occupied(&nothing_reported),
            vec![],
            "no checks reported means every bucket is empty"
        );
        assert_eq!(
            nothing_reported.total(),
            0,
            "no checks reported is a total of none"
        );

        let all_green = classify(
            &checks(&[(REQUIRED, "SUCCESS"), (ADVISORY, "SUCCESS")]),
            &policy,
        );
        assert_eq!(
            occupied(&all_green),
            vec![("passing", vec![REQUIRED, ADVISORY])],
            "checks that passed are reported as passing, not as an absence"
        );
        assert_eq!(
            all_green.total(),
            2,
            "two checks reported is a total of two"
        );
    }

    #[test]
    fn the_total_counts_all_four_buckets_and_counts_a_re_run_check_once() {
        // `total()` sums four buckets, so a rollup that fills one of them
        // cannot tell whether the other three terms are in the sum at all —
        // which is exactly how the two assertions elsewhere in this file leave
        // it: both run with blocking, advisory and running empty. One check in
        // each bucket makes every term load-bearing.
        //
        // And the count is of checks, not attempts. The fifth row is the
        // fourth's re-run, so five attempts must total four.
        let policy = rule_requiring(&[REQUIRED]);
        let classified = classify(
            &attempts(&[
                (REQUIRED, "FAILURE", 0),
                (ADVISORY, "FAILURE", 0),
                ("build / test (ubuntu-latest)", "IN_PROGRESS", 0),
                (REQUIRED_RE_RUN, "FAILURE", 0),
                (REQUIRED_RE_RUN, "SUCCESS", 1),
            ]),
            &policy,
        );

        assert_eq!(
            occupied(&classified),
            vec![
                ("blocking", vec![REQUIRED]),
                ("advisory", vec![ADVISORY]),
                ("running", vec!["build / test (ubuntu-latest)"]),
                ("passing", vec![REQUIRED_RE_RUN]),
            ],
            "one check in each bucket, with the re-run merged onto its name"
        );
        assert_eq!(
            classified.total(),
            4,
            "four distinct checks out of five attempts, summed across all four buckets"
        );
    }

    #[test]
    fn only_passing_checks_leave_nothing_blocking_and_nothing_advisory() {
        let policy = rule_requiring(&[REQUIRED]);
        let classified = classify(
            &checks(&[
                (REQUIRED, "SUCCESS"),
                (ADVISORY, "SKIPPED"),
                ("build / test (ubuntu-latest)", "NEUTRAL"),
            ]),
            &policy,
        );
        assert_eq!(
            classified.blocking().len(),
            0,
            "nothing failed, so nothing can block"
        );
        assert_eq!(
            classified.advisory().len(),
            0,
            "nothing failed, so nothing is advisory either"
        );
        assert_eq!(classified.running().len(), 0, "nothing is still going");
        assert_eq!(classified.passing().len(), 3, "all three checks passed");
    }

    #[test]
    fn every_state_github_reports_parses_to_the_variant_that_names_it() {
        let cases: [(&str, CheckState); 15] = [
            ("FAILURE", CheckState::Failure),
            ("ERROR", CheckState::Error),
            ("TIMED_OUT", CheckState::TimedOut),
            ("STARTUP_FAILURE", CheckState::StartupFailure),
            ("ACTION_REQUIRED", CheckState::ActionRequired),
            ("CANCELLED", CheckState::Cancelled),
            ("STALE", CheckState::Stale),
            ("IN_PROGRESS", CheckState::InProgress),
            ("QUEUED", CheckState::Queued),
            ("PENDING", CheckState::Pending),
            ("WAITING", CheckState::Waiting),
            ("EXPECTED", CheckState::Expected),
            ("SUCCESS", CheckState::Success),
            ("SKIPPED", CheckState::Skipped),
            ("NEUTRAL", CheckState::Neutral),
        ];
        for (raw, expected) in cases {
            assert_eq!(
                CheckState::parse(raw),
                expected,
                "{raw:?} must parse to the state that names it"
            );
        }
    }

    #[test]
    fn a_state_this_crate_does_not_know_keeps_the_text_it_arrived_as() {
        // Preserving the spelling is the entire purpose of `Unknown`, and a
        // probe that is already upper-case and unpadded cannot tell whether it
        // was preserved or rebuilt: `SOMETHING_NEW` survives an implementation
        // that upper-cases the text, and one that forgets to trim. Every row
        // below differs from its normalised form in at least one of those two
        // ways.
        for (raw, kept) in [
            ("SOMETHING_NEW", "SOMETHING_NEW"),
            ("  Something_New\n", "Something_New"),
            ("\tsomething_new ", "something_new"),
            (" Needs Review From Legal ", "Needs Review From Legal"),
        ] {
            assert_eq!(
                CheckState::parse(raw),
                CheckState::Unknown {
                    reported: kept.to_owned()
                },
                "{raw:?} must reach a reader as {kept:?}, neither re-cased nor padded"
            );
        }
    }

    #[test]
    fn a_state_is_read_the_same_however_it_is_cased_or_padded() {
        for raw in [
            "FAILURE",
            "failure",
            "Failure",
            "  FAILURE ",
            "\tfailure\r\n",
        ] {
            assert_eq!(
                CheckState::parse(raw),
                CheckState::Failure,
                "{raw:?} names the same state as FAILURE"
            );
        }
    }
}
