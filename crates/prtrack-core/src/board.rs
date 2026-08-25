//! Which section of the board a pull request belongs in, and where it sorts.

use std::cmp::Reverse;

use crate::checks::{classify, Classified};
use crate::decision::BlockedReason;
use crate::facts::{DraftState, PrFacts};
use crate::policy::PolicyRules;
use crate::types::Login;

/// How soon the viewer should deal with their own pull request.
///
/// The sort key for [`Board::yours`], and **not** the order of the ladder that
/// produced it. The ladder answers "why can this not merge"; this answers "what
/// should I pick up first", and the two disagree at both ends — a pull request
/// that can merge is the ladder's happiest answer and this one's most urgent,
/// because merging it is the single act that unblocks other people.
///
/// Ordering is the declaration order of the variants, so reading them top to
/// bottom is reading the board's opinion of your morning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ActionUrgency {
    /// Nothing is in the way. One click ends someone else's wait.
    Mergeable,
    /// Approved, and out of date with its base is the only thing left.
    ///
    /// Second because it is one button press from [`Mergeable`], and the
    /// ladder only reaches it once a pull request is approved — being behind
    /// is not worth saying while a reviewer is still the bottleneck, since
    /// updating a branch changes not one line of the diff they would read. So
    /// this is never "out of date and also unreviewed"; it is always "this is
    /// the last thing".
    ///
    /// [`Mergeable`]: ActionUrgency::Mergeable
    BehindBase,
    /// Approved and then left to rot until it conflicted. The review has
    /// already been paid for and is being thrown away.
    ApprovedAndConflicting,
    /// Conflicting, without an approval yet.
    Conflicting,
    /// A reviewer asked for changes, so a person is waiting on a person.
    ChangesRequested,
    /// A check the repository requires is failing.
    RequiredCheckFailing,
    /// Review threads are open and the repository will not merge until they
    /// are resolved.
    UnresolvedThreads,
    /// A build is running. Nothing to do but wait for it.
    CiRunning,
    /// GitHub has not finished working out whether the branch merges cleanly.
    MergeabilityNotKnown,
    /// Waiting on other people. The most you can do is go and ask them.
    AwaitingReview,
    /// A draft, which cannot merge whatever else is true of it — last, because
    /// no other row on the board is waiting behind it.
    Draft,
}

impl ActionUrgency {
    /// Reads urgency off the reason a pull request cannot merge.
    ///
    /// Several rungs collapse: an approved conflict and a plain one are the
    /// same job, and a build running is a build running whether or not the
    /// review is already in. The ladder keeps them apart because they read
    /// differently in a sentence; this does not, because they queue the same.
    const fn of(reason: &BlockedReason) -> Self {
        match reason {
            BlockedReason::ReadyToMerge => Self::Mergeable,
            BlockedReason::ApprovedButConflicting => Self::ApprovedAndConflicting,
            BlockedReason::Conflicting => Self::Conflicting,
            BlockedReason::ChangesRequested { .. } => Self::ChangesRequested,
            BlockedReason::RequiredCheckFailing { .. } => Self::RequiredCheckFailing,
            BlockedReason::UnresolvedThreads { .. } => Self::UnresolvedThreads,
            BlockedReason::BehindBase => Self::BehindBase,
            BlockedReason::ApprovedCiRunning | BlockedReason::CiRunning => Self::CiRunning,
            BlockedReason::MergeabilityNotKnown => Self::MergeabilityNotKnown,
            BlockedReason::WaitingForReview { .. } | BlockedReason::NobodyAssigned => {
                Self::AwaitingReview
            }
            BlockedReason::Draft => Self::Draft,
        }
    }
}

/// How much good it would do the viewer to open somebody else's pull request
/// right now.
///
/// The sort key for [`Board::needs_your_review`], and a different question from
/// [`ActionUrgency`]: not "how stuck is this" but "can I help by reading it".
/// A pull request waiting on its own author is not reviewable however green its
/// checks are, because the code you would be reading is about to change. On the
/// live repository that distinction is the difference between a section of
/// thirteen rows and the four somebody can actually pick up, which is the whole
/// reason the key exists.
///
/// Ordering is the declaration order of the variants: most reviewable first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Reviewability {
    /// Nothing is in the way of reading it.
    Ready,
    /// Threads are open that the repository requires resolved. Still readable;
    /// the conversation is part of the review.
    UnresolvedThreads,
    /// It conflicts with its base. Somebody owes a rebase, but the diff is
    /// still worth reading.
    Conflicting,
    /// No check has reported at all, so nothing is known about whether the code
    /// runs. Less reviewable than a conflict, because a conflict is a known
    /// quantity and silence is not.
    NoChecksReported,
    /// A build is still running, so the verdict may yet change.
    CiRunning,
    /// A reviewer already asked for changes, so the author owes the next move
    /// and the code is about to change under you.
    ChangesRequested,
    /// A required check is failing. The author has to fix it before any review
    /// is worth writing.
    RequiredCheckFailing,
}

impl Reviewability {
    /// Reads reviewability off the reason a pull request cannot merge, and off
    /// whether anything has reported at all.
    ///
    /// The blocked reason alone cannot answer this. An empty rollup is not a
    /// rung of the ladder — [`Classified::total`] of zero is a fact about the
    /// checks, not about the merge — so it is consulted separately, and it
    /// outranks everything below it.
    ///
    /// [`Classified::total`]: crate::Classified::total
    fn of(reason: &BlockedReason, checks: &Classified) -> Self {
        let rung = Self::of_reason(reason);
        // Silence puts a ceiling on how reviewable a pull request can be. If
        // the ladder placed it above `NoChecksReported` — readable, or readable
        // with a caveat — an empty rollup drags it down to this, because
        // nothing has established that the code so much as builds. Anything the
        // ladder already placed *below* keeps its own rung, since the author
        // owing a fix outranks not knowing whether one is needed.
        //
        // Asked after the three author-owes rungs and before the rest: a
        // reviewer can act on a pull request whose author owes them nothing,
        // even when nothing has tested it.
        if checks.total() == 0 && rung < Self::NoChecksReported {
            return Self::NoChecksReported;
        }
        rung
    }

    /// The rung the blocked reason alone implies, before an empty rollup has
    /// had its say.
    const fn of_reason(reason: &BlockedReason) -> Self {
        match reason {
            BlockedReason::RequiredCheckFailing { .. } => Self::RequiredCheckFailing,
            BlockedReason::ChangesRequested { .. } => Self::ChangesRequested,
            BlockedReason::ApprovedCiRunning | BlockedReason::CiRunning => Self::CiRunning,
            BlockedReason::Conflicting | BlockedReason::ApprovedButConflicting => Self::Conflicting,
            BlockedReason::UnresolvedThreads { .. } => Self::UnresolvedThreads,
            // `Draft` cannot reach here — drafts are never in the review
            // section — but it is named rather than swept into a wildcard, so
            // that a rung added to the ladder later has to be placed here on
            // purpose instead of silently becoming "perfectly reviewable".
            BlockedReason::ReadyToMerge
            | BlockedReason::WaitingForReview { .. }
            | BlockedReason::NobodyAssigned
            | BlockedReason::BehindBase
            | BlockedReason::MergeabilityNotKnown
            | BlockedReason::Draft => Self::Ready,
        }
    }
}

/// Which of the board's groups a pull request belongs to, from the viewer's
/// point of view.
///
/// Private because [`Board`] is the answer: a caller reads the section a pull
/// request landed in by finding it in one, and the four are a partition so
/// there is always exactly one to find.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    /// The viewer wrote it, and it is open for review.
    Yours,
    /// The viewer wrote it, and it is still a draft.
    ///
    /// Separate from [`Section::Yours`] rather than sorted to the bottom of it,
    /// because the two answer different questions. Your open work is what
    /// somebody is waiting on; your drafts are what you have not asked anybody
    /// for yet, and mixing them makes the first list longer than the thing it
    /// is measuring.
    YourDrafts,
    /// Somebody else's draft.
    OtherDrafts,
    /// Somebody else's work, open for review, with the viewer asked for one.
    NeedsYourReview,
    /// Somebody else's work that is not waiting on the viewer at all.
    NotWaitingOnYou,
}

impl Section {
    /// Places a pull request from the viewer's point of view.
    ///
    /// The order of the questions is the whole definition and is not
    /// rearrangeable. Authorship is asked first, so a review request on your
    /// own pull request cannot list it twice. Draft is asked next, so
    /// somebody else's draft stays a draft even when you have been asked to
    /// review it — a draft is not ready for the review section by definition.
    fn of(facts: &PrFacts, viewer: &Login) -> Self {
        if facts.author.as_ref() == Some(viewer) {
            return match facts.draft_state {
                DraftState::Draft => Self::YourDrafts,
                DraftState::ReadyForReview => Self::Yours,
            };
        }
        match facts.draft_state {
            DraftState::Draft => return Self::OtherDrafts,
            DraftState::ReadyForReview => {}
        }
        // A team request names no account, so it cannot match here even when
        // the viewer is a member. See `Board` for why that is left alone.
        if facts
            .requested_reviewers
            .iter()
            .any(|reviewer| reviewer.login() == Some(viewer))
        {
            return Self::NeedsYourReview;
        }
        Self::NotWaitingOnYou
    }
}

/// One pull request, reduced to everything a board needs to show or sort it.
///
/// The three things travel together because they are one reading of one pull
/// request: the facts as GitHub reported them, those checks sorted against the
/// repository's rule, and the reason the rule and the facts together say it
/// cannot merge. Assembling them separately would let a caller pair checks
/// classified under one policy with a reason decided under another, which is a
/// row that renders a required failure as advisory or the reverse.
///
/// [`BoardRow::reduce`] is the only way to build one, so that cannot be
/// expressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardRow {
    facts: PrFacts,
    checks: Classified,
    reason: BlockedReason,
}

impl BoardRow {
    /// Reduces one pull request against the rule that governs it.
    ///
    /// `policy` is the rule for *this* pull request's base branch, which is not
    /// necessarily the rule for the next one's: a stacked pull request aimed at
    /// another feature branch is governed by nothing, and the caller says so by
    /// passing [`PolicyRules::unprotected`]. Resolving which rule applies is
    /// the caller's job, because only the caller knows the branch patterns.
    ///
    /// [`PolicyRules::unprotected`]: crate::PolicyRules::unprotected
    #[must_use]
    pub fn reduce(facts: PrFacts, policy: &PolicyRules) -> Self {
        let checks = classify(&facts.checks, policy);
        let reason = BlockedReason::decide(&facts, policy);
        Self {
            facts,
            checks,
            reason,
        }
    }

    /// The pull request as GitHub reported it.
    #[must_use]
    pub const fn facts(&self) -> &PrFacts {
        &self.facts
    }

    /// Its checks, sorted against the rule that governs it.
    #[must_use]
    pub const fn checks(&self) -> &Classified {
        &self.checks
    }

    /// Why it cannot merge yet.
    #[must_use]
    pub const fn reason(&self) -> &BlockedReason {
        &self.reason
    }

    /// How soon the viewer should deal with it, if it is theirs.
    ///
    /// Meaningful for a row in [`Board::yours`]. Reading it for somebody
    /// else's pull request is not wrong, it just answers a question nobody
    /// asked.
    #[must_use]
    pub const fn urgency(&self) -> ActionUrgency {
        ActionUrgency::of(&self.reason)
    }

    /// How much good it would do to open it, if it is somebody else's.
    ///
    /// The sort key for [`Board::needs_your_review`], and the one a renderer
    /// reads to dim the rows that are waiting on their author: anything from
    /// [`Reviewability::ChangesRequested`] down is a row you cannot help with
    /// yet, however inviting the rest of it looks.
    #[must_use]
    pub fn reviewability(&self) -> Reviewability {
        Reviewability::of(&self.reason, &self.checks)
    }
}

/// Every open pull request, grouped by what the viewer can do about it.
///
/// # Nothing is dropped
///
/// The four groups partition the input: every row goes in exactly one, and
/// [`Board::total`] equals the number of rows handed in. Three of them are
/// meant to be rendered. The fourth, [`Board::not_waiting_on_you`], usually is
/// not — but it is kept rather than discarded, because a pull request that
/// quietly stopped existing is the failure this crate is built to prevent, and
/// a board that can only report "37 open" while showing nine of them has to get
/// that number from somewhere honest.
///
/// # A team request cannot find the viewer
///
/// [`Board::needs_your_review`] asks whether the viewer is a requested
/// reviewer, and a [`Reviewer::Team`] names no account. With
/// `requiresCodeOwnerReviews` enabled a `CODEOWNERS` team request makes the
/// viewer an implicit reviewer who never appears by name, so this section
/// under-reports. Closing that needs team-membership data this crate does not
/// have and deliberately does not guess at. The pull request is not lost — it
/// lands in [`Board::not_waiting_on_you`] with its reviewers intact — so the
/// gap is a row in the wrong group rather than a row that disappeared.
///
/// [`Reviewer::Team`]: crate::Reviewer::Team
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Board {
    yours: Vec<BoardRow>,
    your_drafts: Vec<BoardRow>,
    needs_your_review: Vec<BoardRow>,
    other_drafts: Vec<BoardRow>,
    not_waiting_on_you: Vec<BoardRow>,
}

impl Board {
    /// Groups and orders every pull request from `viewer`'s point of view.
    ///
    /// `viewer` is the account the board is being drawn for. Comparison folds
    /// case, because [`Login`] does and GitHub identity does.
    ///
    /// [`Login`]: crate::Login
    #[must_use]
    pub fn assemble(rows: Vec<BoardRow>, viewer: &Login) -> Self {
        let mut board = Self {
            yours: Vec::new(),
            your_drafts: Vec::new(),
            needs_your_review: Vec::new(),
            other_drafts: Vec::new(),
            not_waiting_on_you: Vec::new(),
        };

        for row in rows {
            let group = match Section::of(&row.facts, viewer) {
                Section::Yours => &mut board.yours,
                Section::YourDrafts => &mut board.your_drafts,
                Section::NeedsYourReview => &mut board.needs_your_review,
                Section::OtherDrafts => &mut board.other_drafts,
                Section::NotWaitingOnYou => &mut board.not_waiting_on_you,
            };
            group.push(row);
        }

        board
            .yours
            .sort_by_key(|row| (row.urgency(), row.facts.last_push, row.facts.number.get()));
        board.needs_your_review.sort_by_key(|row| {
            (
                row.reviewability(),
                row.facts.last_push,
                row.facts.number.get(),
            )
        });
        // Both draft sections run newest-push first: a draft is a thing you
        // are still working on, so the one you touched last is the one you
        // meant to come back to.
        board
            .your_drafts
            .sort_by_key(|row| (Reverse(row.facts.last_push), row.facts.number.get()));
        board
            .other_drafts
            .sort_by_key(|row| (Reverse(row.facts.last_push), row.facts.number.get()));
        board
            .not_waiting_on_you
            .sort_by_key(|row| row.facts.number.get());

        board
    }

    /// The viewer's own pull requests that are open for review, most urgent
    /// first.
    ///
    /// Ordered by [`ActionUrgency`], then by the oldest push, so that among
    /// equally urgent work the thing that has been waiting longest comes first.
    ///
    /// Drafts are **not** here — see [`Board::your_drafts`]. They were once
    /// sorted to the bottom of this list, which made it longer than the thing
    /// it measures: what somebody else is waiting on.
    #[must_use]
    pub fn yours(&self) -> &[BoardRow] {
        &self.yours
    }

    /// The viewer's own drafts, newest push first.
    ///
    /// Nobody is waiting on these, so they are not ordered by urgency — the
    /// useful order is the one you were last working in.
    #[must_use]
    pub fn your_drafts(&self) -> &[BoardRow] {
        &self.your_drafts
    }

    /// Somebody else's open pull requests with the viewer asked for a review,
    /// most reviewable first.
    ///
    /// Drafts are never here; they are in [`Board::other_drafts`] whether or
    /// not a review was requested.
    #[must_use]
    pub fn needs_your_review(&self) -> &[BoardRow] {
        &self.needs_your_review
    }

    /// Somebody else's drafts, newest push first.
    ///
    /// Present for completeness rather than for action: nobody is expected to
    /// do anything about a draft that is not theirs.
    #[must_use]
    pub fn other_drafts(&self) -> &[BoardRow] {
        &self.other_drafts
    }

    /// Everything else — somebody else's open work that has not been put in
    /// front of the viewer.
    ///
    /// Ordered by pull request number, since no urgency applies. Usually
    /// counted rather than rendered; see the type's documentation for why it is
    /// kept at all.
    #[must_use]
    pub fn not_waiting_on_you(&self) -> &[BoardRow] {
        &self.not_waiting_on_you
    }

    /// How many pull requests the board was built from.
    ///
    /// Equal to the number of rows handed to [`Board::assemble`], because the
    /// four groups are a partition of them.
    #[must_use]
    pub const fn total(&self) -> usize {
        self.yours.len()
            + self.your_drafts.len()
            + self.needs_your_review.len()
            + self.other_drafts.len()
            + self.not_waiting_on_you.len()
    }
}

#[cfg(test)]
mod tests {
    use super::{Board, BoardRow};
    use crate::checks::{Check, CheckState};
    use crate::facts::{
        BranchName, DraftState, MergeState, MergeableState, PrFacts, PrTitle, PrUrl,
        ReviewDecision, Reviewer, TeamSlug,
    };
    use crate::policy::{PolicyRules, Requirement};
    use crate::types::{CheckName, HeadSha, InvalidValue, Login, PrNumber, RepoRef};
    use chrono::{DateTime, TimeDelta, Utc};

    /// An object id, distinct per pull request only because nothing here reads
    /// it. No sort key or section depends on the head commit.
    const SOME_SHA: &str = "9f2c4e1b7a3d5068c9e4b2a17f36d8095c1e4a7b";

    /// An instant `minutes` from an arbitrary base. No test cares what the
    /// wall-clock value is, only which of two pushes is the older.
    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::<Utc>::UNIX_EPOCH + TimeDelta::minutes(minutes)
    }

    /// An open pull request with nothing wrong with it, for a test to spoil one
    /// field at a time.
    ///
    /// Built through the real constructors, so a fixture that stopped parsing
    /// fails loudly rather than leaving the assertions asserting nothing.
    fn open_pr(number: u32, author: &str) -> Result<PrFacts, InvalidValue> {
        Ok(PrFacts {
            repo: RepoRef::parse("Example-Org/example-repo")?,
            number: PrNumber::new(number)?,
            url: PrUrl::new("https://github.com/Example-Org/example-repo/pull/1"),
            title: PrTitle::new("Split the ledger writer"),
            author: Some(Login::parse(author)?),
            draft_state: DraftState::ReadyForReview,
            head: HeadSha::parse(SOME_SHA)?,
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

    /// The pull request numbers a section holds, in the order it holds them.
    fn numbers(rows: &[BoardRow]) -> Vec<u32> {
        rows.iter().map(|row| row.facts().number.get()).collect()
    }

    /// Reduces every pull request against the same rule and arranges the board.
    fn board_for(prs: Vec<PrFacts>, viewer: &Login) -> Board {
        let policy = PolicyRules::unprotected();
        let rows = prs
            .into_iter()
            .map(|facts| BoardRow::reduce(facts, &policy))
            .collect();
        Board::assemble(rows, viewer)
    }

    /// A check attempt, so a fixture can be given a rollup worth classifying.
    fn check(name: &str, state: &str) -> Result<Check, InvalidValue> {
        Ok(Check::new(
            CheckName::parse(name)?,
            CheckState::parse(state),
            Some(at(0)),
        ))
    }

    /// A rule that demands everything the ladder can ask about, so one board
    /// can exercise the rungs that only exist under a strict policy.
    fn demanding_rule() -> PolicyRules {
        PolicyRules {
            conversation_resolution: Requirement::Required,
            up_to_date_branch: Requirement::Required,
            ..PolicyRules::unprotected()
        }
    }

    /// Reduces against `policy` rather than against no rule at all.
    fn board_under(policy: &PolicyRules, prs: Vec<PrFacts>, viewer: &Login) -> Board {
        let rows = prs
            .into_iter()
            .map(|facts| BoardRow::reduce(facts, policy))
            .collect();
        Board::assemble(rows, viewer)
    }

    #[test]
    fn your_own_pull_requests_run_from_most_to_least_worth_picking_up() {
        // Every rung of the urgency ladder in one ordering, and the two ends
        // are the point: a pull request that can merge is the *most* urgent
        // thing on your list, because one click ends somebody else's wait —
        // while the ladder that produced it calls that the happiest answer it
        // has. The draft in this fixture is *not* in the result: it belongs to
        // `your_drafts` now, and its absence here is the point.
        //
        // Numbers descend and pushes descend as urgency worsens, so the
        // expected order is neither the number order nor the push order. Both
        // are what this section falls back on when the urgency key is not
        // consulted.
        let order = Login::parse("octocat").and_then(|me| {
            let mut mergeable = open_pr(110, "octocat")?;
            mergeable.review_decision = Some(ReviewDecision::Approved);
            mergeable.last_push = at(100);

            // Approved on purpose: the ladder places `BehindBase` inside the
            // approval, so an unapproved pull request that is behind never
            // reaches the rung at all. An earlier draft of this fixture left
            // the approval off and the row landed in `AwaitingReview`, which
            // is what sent me to read the ladder rather than the plan.
            let mut behind = open_pr(100, "octocat")?;
            behind.review_decision = Some(ReviewDecision::Approved);
            behind.merge_state = MergeState::Behind;
            behind.last_push = at(90);

            let mut approved_conflicting = open_pr(90, "octocat")?;
            approved_conflicting.review_decision = Some(ReviewDecision::Approved);
            approved_conflicting.mergeable = MergeableState::Conflicting;
            approved_conflicting.last_push = at(80);

            let mut conflicting = open_pr(80, "octocat")?;
            conflicting.mergeable = MergeableState::Conflicting;
            conflicting.last_push = at(70);

            let mut changes_requested = open_pr(70, "octocat")?;
            changes_requested.changes_requested_by = vec![Login::parse("hubot")?];
            changes_requested.last_push = at(60);

            let mut failing = open_pr(60, "octocat")?;
            failing.checks = vec![check("Preview CI Gate", "FAILURE")?];
            failing.last_push = at(50);

            let mut threads_open = open_pr(50, "octocat")?;
            threads_open.unresolved_threads = 2;
            threads_open.last_push = at(40);

            let mut building = open_pr(40, "octocat")?;
            building.checks = vec![check("Preview CI Gate", "IN_PROGRESS")?];
            building.last_push = at(30);

            let mut computing = open_pr(30, "octocat")?;
            computing.mergeable = MergeableState::Computing;
            computing.last_push = at(20);

            let mut awaiting_review = open_pr(20, "octocat")?;
            awaiting_review.last_push = at(10);

            let mut draft = open_pr(10, "octocat")?;
            draft.draft_state = DraftState::Draft;
            draft.last_push = at(0);

            let board = board_under(
                &demanding_rule(),
                vec![
                    draft,
                    awaiting_review,
                    computing,
                    building,
                    threads_open,
                    failing,
                    changes_requested,
                    conflicting,
                    approved_conflicting,
                    behind,
                    mergeable,
                ],
                &me,
            );
            Ok::<_, InvalidValue>(numbers(board.yours()))
        });

        assert_eq!(
            order,
            Ok(vec![110, 100, 90, 80, 70, 60, 50, 40, 30, 20]),
            "mergeable first, then the one-click fix, draft last"
        );
    }

    #[test]
    fn your_draft_is_its_own_section_however_healthy_it_looks() {
        // The draft is approved with a clean merge -- everything that would
        // otherwise make it the most urgent row on the board -- and it is still
        // not in the list of work somebody is waiting on, because a draft is
        // not asking anybody for anything.
        //
        // It used to sort to the bottom of `yours` instead. That made the list
        // longer than the thing it measures, and a count of "your open work"
        // that includes drafts answers a question nobody asked.
        let split = Login::parse("octocat").and_then(|me| {
            let mut healthy_draft = open_pr(1, "octocat")?;
            healthy_draft.draft_state = DraftState::Draft;
            healthy_draft.review_decision = Some(ReviewDecision::Approved);
            healthy_draft.last_push = at(0);

            let mut ordinary = open_pr(2, "octocat")?;
            ordinary.last_push = at(10);

            let board = board_for(vec![healthy_draft, ordinary], &me);
            Ok::<_, InvalidValue>((numbers(board.yours()), numbers(board.your_drafts())))
        });

        assert_eq!(
            split,
            Ok((vec![2], vec![1])),
            "the draft is yours, and it is not open work"
        );
    }

    #[test]
    fn your_drafts_run_newest_first_like_everybody_else_s() {
        // Nobody is waiting on a draft, so urgency is the wrong order for them.
        // The useful one is the order you were last working in.
        let order = Login::parse("octocat").and_then(|me| {
            let mut oldest = open_pr(1, "octocat")?;
            oldest.draft_state = DraftState::Draft;
            oldest.last_push = at(0);

            let mut newest = open_pr(2, "octocat")?;
            newest.draft_state = DraftState::Draft;
            newest.last_push = at(100);

            let board = board_for(vec![oldest, newest], &me);
            Ok::<_, InvalidValue>(numbers(board.your_drafts()))
        });

        assert_eq!(
            order,
            Ok(vec![2, 1]),
            "the one you touched last comes first"
        );
    }

    #[test]
    fn other_peoples_drafts_run_newest_first() {
        // The one section that reads backwards from the others. Nobody is
        // expected to act on somebody else's draft, so the useful ordering is
        // "what has just appeared" rather than "what has been waiting".
        //
        // Numbers descend as the push gets older, so neither ascending order
        // would produce this.
        let order = Login::parse("octocat").and_then(|me| {
            let mut newest = open_pr(30, "hubot")?;
            newest.draft_state = DraftState::Draft;
            newest.last_push = at(30);

            let mut middle = open_pr(20, "hubot")?;
            middle.draft_state = DraftState::Draft;
            middle.last_push = at(20);

            let mut oldest = open_pr(10, "hubot")?;
            oldest.draft_state = DraftState::Draft;
            oldest.last_push = at(10);

            let board = board_for(vec![oldest, newest, middle], &me);
            Ok::<_, InvalidValue>(numbers(board.other_drafts()))
        });

        assert_eq!(
            order,
            Ok(vec![30, 20, 10]),
            "the most recently pushed draft comes first"
        );
    }

    #[test]
    fn two_rows_that_tie_on_every_key_are_ordered_by_number_whatever_order_they_arrived_in() {
        // Ties are real: two pull requests can share a section, a rung, and a
        // push instant. Leaving them to the sort's stability would make the
        // board depend on the order GitHub happened to return, so the same two
        // rows could swap places between two runs that fetched identical data.
        //
        // The pull request number breaks it — a total order over data the board
        // already holds. Asserted by feeding the same set both ways round and
        // requiring the same answer, which a stable sort alone cannot give.
        let both_ways = Login::parse("octocat").and_then(|me| {
            let pair = |first: u32, second: u32| -> Result<Vec<u32>, InvalidValue> {
                let mut earlier = open_pr(first, "octocat")?;
                earlier.last_push = at(5);
                let mut later = open_pr(second, "octocat")?;
                later.last_push = at(5);

                let board = board_for(vec![earlier, later], &me);
                Ok(numbers(board.yours()))
            };
            Ok::<_, InvalidValue>((pair(3, 5)?, pair(5, 3)?))
        });

        assert_eq!(
            both_ways,
            Ok((vec![3, 5], vec![3, 5])),
            "the lower number wins the tie, and the input order does not get a say"
        );
    }

    #[test]
    fn a_pull_request_waiting_on_its_own_author_is_the_last_thing_worth_reviewing() {
        // The case the section exists for. `#7` is green all the way through
        // and `#8`'s build has not even finished — but `#7` is waiting on its
        // author to answer a review, so opening it achieves nothing. On the
        // live repository this is the difference between a section of thirteen
        // rows and the four that can actually be picked up.
        let order = Login::parse("octocat").and_then(|me| {
            let asked = vec![Reviewer::User(me.clone())];

            let mut blocked_on_author = open_pr(7, "hubot")?;
            blocked_on_author.requested_reviewers = asked.clone();
            blocked_on_author.changes_requested_by = vec![Login::parse("someone-else")?];
            blocked_on_author.checks = vec![check("Preview CI Gate", "SUCCESS")?];

            let mut still_building = open_pr(8, "hubot")?;
            still_building.requested_reviewers = asked;
            still_building.checks = vec![check("Preview CI Gate", "IN_PROGRESS")?];

            let board = board_for(vec![blocked_on_author, still_building], &me);
            Ok::<_, InvalidValue>(numbers(board.needs_your_review()))
        });

        assert_eq!(
            order,
            Ok(vec![8, 7]),
            "a pull request blocked on its author sorts last however green its checks are"
        );
    }

    #[test]
    fn the_review_section_runs_from_most_reviewable_to_least() {
        // The whole reviewability ladder in one ordering. Each fixture differs
        // only in the one fact that decides its rung, and the assertion is on
        // the order rather than on each key, because the order is the thing a
        // reader depends on.
        //
        // The numbers descend and the pushes ascend as reviewability worsens,
        // so the expected order is neither the number order nor the push order.
        // Both of those are what this section would fall back on if the
        // reviewability key stopped being consulted, and an earlier draft of
        // this test passed against exactly that — the expected order happened
        // to be `1..6`, which is what sorting by number alone produces.
        let order = Login::parse("octocat").and_then(|me| {
            let asked = vec![Reviewer::User(me.clone())];
            let green = vec![check("Preview CI Gate", "SUCCESS")?];

            let mut ready = open_pr(60, "hubot")?;
            ready.requested_reviewers = asked.clone();
            ready.checks = green.clone();
            ready.last_push = at(50);

            let mut conflicting = open_pr(50, "hubot")?;
            conflicting.requested_reviewers = asked.clone();
            conflicting.checks = green.clone();
            conflicting.mergeable = MergeableState::Conflicting;
            conflicting.last_push = at(40);

            let mut nothing_reported = open_pr(40, "hubot")?;
            nothing_reported.requested_reviewers = asked.clone();
            nothing_reported.last_push = at(30);

            let mut building = open_pr(30, "hubot")?;
            building.requested_reviewers = asked.clone();
            building.checks = vec![check("Preview CI Gate", "IN_PROGRESS")?];
            building.last_push = at(20);

            let mut changes_requested = open_pr(20, "hubot")?;
            changes_requested.requested_reviewers = asked.clone();
            changes_requested.checks = green;
            changes_requested.changes_requested_by = vec![Login::parse("someone-else")?];
            changes_requested.last_push = at(10);

            let mut failing = open_pr(10, "hubot")?;
            failing.requested_reviewers = asked;
            failing.checks = vec![check("Preview CI Gate", "FAILURE")?];
            failing.last_push = at(0);

            let board = board_for(
                vec![
                    failing,
                    changes_requested,
                    building,
                    nothing_reported,
                    conflicting,
                    ready,
                ],
                &me,
            );
            Ok::<_, InvalidValue>(numbers(board.needs_your_review()))
        });

        assert_eq!(
            order,
            Ok(vec![60, 50, 40, 30, 20, 10]),
            "clean first, then conflicting, silent CI, running CI, changes requested, failing"
        );
    }

    #[test]
    fn silence_from_ci_does_not_excuse_a_pull_request_waiting_on_its_author() {
        // An empty rollup caps how reviewable a pull request can look, but it
        // is a ceiling and not a verdict: a pull request whose author owes a
        // reply is *less* reviewable than one nothing has tested, because only
        // one of them has somebody already waiting on a change to the diff.
        //
        // Neither fixture reports a check, so the cap applies to both and only
        // the author-owes rung can separate them. An implementation that let
        // silence overwrite every rung would order these by number and pass.
        let order = Login::parse("octocat").and_then(|me| {
            let asked = vec![Reviewer::User(me.clone())];

            let mut owes_a_reply = open_pr(1, "hubot")?;
            owes_a_reply.requested_reviewers = asked.clone();
            owes_a_reply.changes_requested_by = vec![Login::parse("someone-else")?];

            let mut merely_untested = open_pr(2, "hubot")?;
            merely_untested.requested_reviewers = asked;

            let board = board_for(vec![owes_a_reply, merely_untested], &me);
            Ok::<_, InvalidValue>(numbers(board.needs_your_review()))
        });

        assert_eq!(
            order,
            Ok(vec![2, 1]),
            "an untested pull request still beats one whose author owes a reply"
        );
    }

    #[test]
    fn a_conflicting_pull_request_that_reported_no_checks_is_judged_on_the_silence() {
        // An empty rollup outranks a conflict, which reads backwards until you
        // notice what each says about *reviewing*. A conflict is somebody's
        // rebase and the diff is still readable; CI that never reported means
        // nobody knows whether the code runs, and reviewing it may be
        // premature -- which is asked before conflicts, because an untested
        // conflict is still untested.
        let order = Login::parse("octocat").and_then(|me| {
            let asked = vec![Reviewer::User(me.clone())];

            let mut conflicting_and_silent = open_pr(1, "hubot")?;
            conflicting_and_silent.requested_reviewers = asked.clone();
            conflicting_and_silent.mergeable = MergeableState::Conflicting;

            let mut conflicting_but_tested = open_pr(2, "hubot")?;
            conflicting_but_tested.requested_reviewers = asked;
            conflicting_but_tested.mergeable = MergeableState::Conflicting;
            conflicting_but_tested.checks = vec![check("Preview CI Gate", "SUCCESS")?];

            let board = board_for(vec![conflicting_and_silent, conflicting_but_tested], &me);
            Ok::<_, InvalidValue>(numbers(board.needs_your_review()))
        });

        assert_eq!(
            order,
            Ok(vec![2, 1]),
            "a conflict you can still read beats a pull request nothing has tested"
        );
    }

    #[test]
    fn every_pull_request_lands_in_exactly_one_section_and_none_is_dropped() {
        // The board shows three sections and counts a fourth group it does not
        // show. A pull request that fell out of all four would vanish silently,
        // which is the one failure this crate exists to prevent — so the four
        // are a partition, and that is asserted as a partition rather than as
        // four separate memberships.
        let viewer = Login::parse("octocat");
        let assembled = viewer.and_then(|me| {
            let mut yours_draft = open_pr(1, "octocat")?;
            yours_draft.draft_state = DraftState::Draft;

            let mut asked_of_you = open_pr(3, "hubot")?;
            asked_of_you.requested_reviewers = vec![Reviewer::User(me.clone())];

            let mut someone_elses_draft = open_pr(4, "hubot")?;
            someone_elses_draft.draft_state = DraftState::Draft;

            let prs = vec![
                yours_draft,
                open_pr(2, "OctoCat")?, // same account, different spelling
                asked_of_you,
                someone_elses_draft,
                // Nothing to do with you, and handed over highest-first so
                // that the group's ordering is asserted rather than inherited
                // from the order GitHub happened to return them in.
                open_pr(6, "hubot")?,
                open_pr(5, "hubot")?,
            ];
            let board = board_for(prs, &me);

            Ok::<_, InvalidValue>((
                numbers(board.yours()),
                numbers(board.your_drafts()),
                numbers(board.needs_your_review()),
                numbers(board.other_drafts()),
                numbers(board.not_waiting_on_you()),
                board.total(),
            ))
        });

        assert_eq!(
            assembled,
            Ok((vec![2], vec![1], vec![3], vec![4], vec![5, 6], 6)),
            "each pull request belongs to exactly one section, and all six are still on the board"
        );
    }

    #[test]
    fn a_pull_request_you_wrote_and_were_asked_to_review_is_only_ever_yours() {
        // GitHub lets an author be added as a reviewer of their own work. It
        // must not appear twice: a row in both sections reads as two pieces of
        // work, and the review section is meant to be the list you can act on
        // for somebody else.
        let placed = Login::parse("octocat").and_then(|me| {
            let mut both = open_pr(7, "octocat")?;
            both.requested_reviewers = vec![Reviewer::User(me.clone())];

            let board = board_for(vec![both], &me);
            Ok::<_, InvalidValue>((
                numbers(board.yours()),
                numbers(board.needs_your_review()),
                board.total(),
            ))
        });

        assert_eq!(
            placed,
            Ok((vec![7], Vec::new(), 1)),
            "authorship wins over a review request on your own pull request"
        );
    }

    #[test]
    fn a_review_asked_only_of_a_team_does_not_put_the_pull_request_in_front_of_you() {
        // `Reviewer::Team` names no account, so the viewer cannot be recognised
        // in it even when they are a member. That is a known hole needing
        // team-membership data this crate does not have.
        //
        // What must not happen is the request vanishing. The pull request stays
        // counted, stays on the board, and keeps its reviewer — so the hole is
        // an under-report that can be found, not a row that disappeared.
        //
        // The slug is deliberately the viewer's own handle. A team and a user
        // can carry the same text, and the only thing keeping them apart is
        // that the membership test reads `Reviewer::login`, which a team has
        // none of. A probe whose slug could not collide with the viewer — an
        // earlier draft used `ledger-owners` against `octocat` — passes just as
        // happily against a test that compares rendered names, and so proves
        // nothing about the distinction it is named for.
        let placed = Login::parse("octocat").and_then(|me| {
            let mut team_only = open_pr(8, "hubot")?;
            team_only.requested_reviewers = vec![Reviewer::Team(TeamSlug::new("octocat"))];

            let board = board_for(vec![team_only], &me);
            let still_named = board
                .not_waiting_on_you()
                .iter()
                .flat_map(|row| row.facts().requested_reviewers.iter())
                .filter_map(Reviewer::name)
                .map(str::to_owned)
                .collect::<Vec<_>>();

            Ok::<_, InvalidValue>((
                numbers(board.needs_your_review()),
                numbers(board.not_waiting_on_you()),
                board.total(),
                still_named,
            ))
        });

        assert_eq!(
            placed,
            Ok((Vec::new(), vec![8], 1, vec!["octocat".to_owned()])),
            "a team request is not a request to you, but it is still a request and still counted"
        );
    }
}
