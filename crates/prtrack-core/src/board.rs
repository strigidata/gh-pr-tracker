//! Which section of the board a pull request belongs in, and where it sorts.

use std::cmp::Reverse;
use std::collections::HashMap;

use crate::checks::{classify, Classified};
use crate::decision::BlockedReason;
use crate::facts::{BranchName, DraftState, HeadRepository, PrFacts, Verdict};
use crate::policy::PolicyRules;
use crate::types::{Login, PrNumber};

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
    /// Nothing is in the way and nothing is failing. Open this one.
    Ready,
    /// Nothing is in the way, but something failed that cannot block the merge.
    ///
    /// Below [`Reviewability::Ready`] because an advisory failure is still a
    /// failure somebody may want to look at, so a pull request with none is the
    /// better use of the next five minutes. Above everything else, because
    /// nothing here stops the review being written.
    ReadyWithAdvisory,
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
        // A pull request with nothing in its way but something red beside it is
        // still readable, and still second to one with nothing red at all.
        // Asked only of `Ready`, so it cannot promote a rung below it.
        if rung == Self::Ready && !checks.advisory().is_empty() {
            return Self::ReadyWithAdvisory;
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
/// request landed in by finding it in one, and the six are a partition so
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
    /// Somebody else's work, open for review, that the viewer has neither been
    /// asked about nor already reviewed.
    ///
    /// Separate from [`Section::YouReviewed`] because it is the one group
    /// there that somebody could usefully pick up. A pull request the viewer
    /// has already approved is *not* here: GitHub removes a reviewer from the
    /// request list the moment they submit, so without checking who has already
    /// reviewed, approved work is indistinguishable from unclaimed work.
    Unclaimed,
    /// Somebody else's work, open for review, that the viewer has already
    /// approved or asked changes on.
    YouReviewed,
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
        // Had your say, and the author has pushed since: the code you judged
        // is gone, so the next move is yours again. Exact rather than by the
        // clock — see `PrFacts::rereview_due` for why a review GitHub did not
        // place on a commit never counts as pushed past.
        if facts.rereview_due(viewer).is_some() {
            return Self::NeedsYourReview;
        }
        // Already had your say on the head — approved it, or asked for
        // changes. The next move is somebody else's, and showing it as
        // reviewable would be asking you to do a thing you have done.
        if facts.reviews.iter().any(|review| &review.by == viewer) {
            return Self::YouReviewed;
        }
        Self::Unclaimed
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
    /// The rule the reason was decided under, kept so the board can put a
    /// viewer's question about the same facts to the same rule.
    policy: PolicyRules,
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
            policy: policy.clone(),
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

    /// Why it could not merge once `viewer`'s own change request is answered.
    ///
    /// The ladder decided on the facts with the viewer taken out of
    /// [`PrFacts::changes_requested_by`], under the same rule as
    /// [`BoardRow::reason`]. Only the viewer is removed: a request somebody
    /// else made is not answered by a push the viewer has yet to read.
    fn reason_once_answered(&self, viewer: &Login) -> BlockedReason {
        let mut facts = self.facts.clone();
        facts.changes_requested_by.retain(|login| login != viewer);
        BlockedReason::decide(&facts, &self.policy)
    }
}

/// One pull request, and every pull request stacked on top of it.
///
/// A tree rather than a list: two branches can be cut from the same feature
/// branch, and each becomes a pull request whose base is that branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stack {
    row: BoardRow,
    /// Where this row would sit on its own. A stacked row is drawn in its
    /// root's section, and the counts under the board still read this.
    own: Section,
    /// The viewer's verdict the author has pushed past, where that is what
    /// put the row in front of them; `None` everywhere else.
    rereview: Option<Verdict>,
    /// Why it cannot merge, as the viewer should read it: past their own
    /// answered change request on a re-review, the row's reason otherwise.
    reason: BlockedReason,
    above: Vec<Self>,
}

impl Stack {
    /// The pull request at the bottom of this stack.
    #[must_use]
    pub const fn row(&self) -> &BoardRow {
        &self.row
    }

    /// The pull requests whose base is this one's branch, by number.
    #[must_use]
    pub fn above(&self) -> &[Self] {
        &self.above
    }

    fn len(&self) -> usize {
        1 + self.above.iter().map(Self::len).sum::<usize>()
    }

    /// How much good opening this row would do the viewer, read off the
    /// viewer-adjusted reason. The sort key of both review groups.
    fn reviewability(&self) -> Reviewability {
        Reviewability::of(&self.reason, &self.row.checks)
    }

    fn walk_into<'a>(&'a self, position: Position, out: &mut Vec<Placed<'a>>) {
        out.push(Placed {
            stack: self,
            position,
        });
        let depth = position.depth().saturating_add(1);
        for stack in &self.above {
            stack.walk_into(
                Position::Stacked {
                    on: self.row.facts.number,
                    depth,
                },
                out,
            );
        }
    }
}

/// Where a row sits in its tree.
///
/// An enum so that "stacked on nothing at depth three" cannot be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Position {
    Root,
    Stacked { on: PrNumber, depth: usize },
}

impl Position {
    const fn depth(self) -> usize {
        match self {
            Self::Root => 0,
            Self::Stacked { depth, .. } => depth,
        }
    }
}

/// One row as a reader meets it: in pre-order, with how far up a stack it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed<'a> {
    stack: &'a Stack,
    position: Position,
}

impl<'a> Placed<'a> {
    /// The pull request.
    #[must_use]
    pub const fn row(&self) -> &'a BoardRow {
        &self.stack.row
    }

    /// How many pull requests it is stacked on. Zero for a root.
    #[must_use]
    pub const fn depth(&self) -> usize {
        self.position.depth()
    }

    /// The pull request directly beneath it, or `None` for a root.
    #[must_use]
    pub const fn stacked_on(&self) -> Option<PrNumber> {
        match self.position {
            Position::Root => None,
            Position::Stacked { on, .. } => Some(on),
        }
    }

    /// The verdict the viewer gave before the author pushed again, when that
    /// is why this row is in front of them.
    ///
    /// `None` for a row the viewer has not had a say on, has had a say on at
    /// the head, or that is not somebody else's open pull request. See
    /// [`PrFacts::rereview_due`] for when a review counts as pushed past.
    #[must_use]
    pub const fn rereview(&self) -> Option<Verdict> {
        self.stack.rereview
    }

    /// How much good it would do the viewer to open this row now.
    ///
    /// [`BoardRow::reviewability`] as the viewer should read it. On a
    /// re-review the viewer's own change request is what the push answered,
    /// so it is set aside and only what else blocks the row counts; without
    /// that, a re-review would be dimmed and sorted last by the very request
    /// it asks the viewer to look at again. Identical to
    /// [`BoardRow::reviewability`] on every other row.
    #[must_use]
    pub fn reviewability(&self) -> Reviewability {
        self.stack.reviewability()
    }
}

/// One section of the board: its stacks, roots in the section's order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Stacks(Vec<Stack>);

impl Stacks {
    /// The trees, roots in order.
    #[must_use]
    pub fn stacks(&self) -> &[Stack] {
        &self.0
    }

    /// How many pull requests, counting every row in every tree.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.iter().map(Stack::len).sum()
    }

    /// Whether the section has no pull requests at all.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Every row in pre-order: each root, then what is stacked on it.
    #[must_use]
    pub fn walk(&self) -> Vec<Placed<'_>> {
        let mut out = Vec::with_capacity(self.len());
        for stack in &self.0 {
            stack.walk_into(Position::Root, &mut out);
        }
        out
    }

    /// Every row in pre-order, without its position.
    pub fn rows(&self) -> impl Iterator<Item = &BoardRow> {
        self.walk().into_iter().map(|placed| placed.row())
    }
}

/// Every open pull request, grouped by what the viewer can do about it.
///
/// Each group is a [`Stacks`]: every stack *rooted* there, held as a tree so
/// that a pull request whose base is another's head is drawn above it. A pull
/// request stacked on nothing is a tree of one. A stacked row is drawn in its
/// root's section, not its own, so a stack is never split across the board;
/// the counts ([`Board::reviewable_now`], [`Board::you_have_reviewed`]) still
/// read where each row would sit on its own.
///
/// # Nothing is dropped
///
/// The six groups partition the input: every row is drawn in exactly one, and
/// [`Board::total`] equals the number of rows handed in. All six are rendered,
/// [`Board::you_reviewed`] included, and none is a bin for rows that fit
/// nowhere else: a pull request that quietly stopped existing is the failure
/// this crate is built to prevent, and a board that reports "37 open" has to
/// get that number from the rows it actually holds.
///
/// # A team request cannot find the viewer
///
/// [`Board::needs_your_review`] asks whether the viewer is a requested
/// reviewer, and a [`Reviewer::Team`] names no account. With
/// `requiresCodeOwnerReviews` enabled a `CODEOWNERS` team request makes the
/// viewer an implicit reviewer who never appears by name, so this section
/// under-reports. Closing that needs team-membership data this crate does not
/// have and deliberately does not guess at. The pull request is not lost — it
/// lands in [`Board::unclaimed`] with its reviewers intact — so the
/// gap is a row in the wrong group rather than a row that disappeared.
///
/// [`Reviewer::Team`]: crate::Reviewer::Team
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Board {
    yours: Stacks,
    unclaimed: Stacks,
    your_drafts: Stacks,
    needs_your_review: Stacks,
    other_drafts: Stacks,
    you_reviewed: Stacks,
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
        let parents = parents(&rows);
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); rows.len()];
        for (child, parent) in parents.iter().enumerate() {
            if let Some(below) = parent.and_then(|parent| children.get_mut(parent)) {
                below.push(child);
            }
        }
        let mut slots: Vec<Option<BoardRow>> = rows.into_iter().map(Some).collect();

        let mut board = Self::default();
        for (root, parent) in parents.iter().enumerate() {
            if parent.is_some() {
                continue;
            }
            if let Some(stack) = grow(root, &mut slots, &children, viewer) {
                board.group_mut(stack.own).0.push(stack);
            }
        }
        board.sort();
        board
    }

    const fn group_mut(&mut self, section: Section) -> &mut Stacks {
        match section {
            Section::Yours => &mut self.yours,
            Section::YourDrafts => &mut self.your_drafts,
            Section::NeedsYourReview => &mut self.needs_your_review,
            Section::Unclaimed => &mut self.unclaimed,
            Section::OtherDrafts => &mut self.other_drafts,
            Section::YouReviewed => &mut self.you_reviewed,
        }
    }

    /// Orders each group's roots by the question that group asks.
    fn sort(&mut self) {
        self.yours.0.sort_by_key(|stack| {
            (
                stack.row.urgency(),
                stack.row.facts.last_push,
                stack.row.facts.number.get(),
            )
        });
        self.needs_your_review.0.sort_by_key(|stack| {
            (
                stack.reviewability(),
                stack.row.facts.last_push,
                stack.row.facts.number.get(),
            )
        });
        // Both draft sections run newest-push first: a draft is a thing you
        // are still working on, so the one you touched last is the one you
        // meant to come back to.
        self.your_drafts.0.sort_by_key(|stack| {
            (
                Reverse(stack.row.facts.last_push),
                stack.row.facts.number.get(),
            )
        });
        self.other_drafts.0.sort_by_key(|stack| {
            (
                Reverse(stack.row.facts.last_push),
                stack.row.facts.number.get(),
            )
        });
        // Not by number. These are rendered under the review section, so they
        // are ordered by the question that section asks: what would it help
        // most for somebody to pick up? Among equally reviewable rows, the one
        // that has sat longest comes first.
        self.unclaimed.0.sort_by_key(|stack| {
            (
                stack.reviewability(),
                stack.row.facts.last_push,
                stack.row.facts.number.get(),
            )
        });
        self.you_reviewed
            .0
            .sort_by_key(|stack| stack.row.facts.number.get());
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
    pub const fn yours(&self) -> &Stacks {
        &self.yours
    }

    /// The viewer's own drafts, newest push first.
    ///
    /// Nobody is waiting on these, so they are not ordered by urgency — the
    /// useful order is the one you were last working in.
    #[must_use]
    pub const fn your_drafts(&self) -> &Stacks {
        &self.your_drafts
    }

    /// Somebody else's open pull requests with the viewer asked for a review,
    /// most reviewable first.
    ///
    /// Drafts are never here; they are in [`Board::other_drafts`] whether or
    /// not a review was requested.
    #[must_use]
    pub const fn needs_your_review(&self) -> &Stacks {
        &self.needs_your_review
    }

    /// Somebody else's drafts, newest push first.
    ///
    /// Present for completeness rather than for action: nobody is expected to
    /// do anything about a draft that is not theirs.
    #[must_use]
    pub const fn other_drafts(&self) -> &Stacks {
        &self.other_drafts
    }

    /// Somebody else's open work that nobody has asked the viewer about, and
    /// that the viewer has not already reviewed.
    ///
    /// Rendered under [`Board::needs_your_review`] rather than hidden, because
    /// "ready for review and nobody assigned" is the state that stalls longest
    /// and it is invisible everywhere else.
    ///
    /// No row here claims the viewer was asked for anything; each one's reason
    /// says `NOBODY ASSIGNED` or names whoever *was*, so the two groups stay
    /// distinguishable without a sub-heading.
    #[must_use]
    pub const fn unclaimed(&self) -> &Stacks {
        &self.unclaimed
    }

    /// Stacks rooted on somebody else's pull request the viewer has already
    /// approved or asked changes on, ordered by number.
    ///
    /// A row stacked on such a pull request is drawn here with it, whether or
    /// not the viewer has reviewed that row too; [`Board::you_have_reviewed`]
    /// and [`Board::reviewable_now`] count each row where it would sit alone.
    ///
    /// GitHub removes a reviewer from the request list the moment they submit,
    /// so without checking who has reviewed, work the viewer approved would be
    /// indistinguishable from work nobody has touched.
    #[must_use]
    pub const fn you_reviewed(&self) -> &Stacks {
        &self.you_reviewed
    }

    /// Every row on the board, section by section in drawing order.
    pub fn walk(&self) -> impl Iterator<Item = Placed<'_>> {
        [
            &self.needs_your_review,
            &self.unclaimed,
            &self.you_reviewed,
            &self.yours,
            &self.your_drafts,
            &self.other_drafts,
        ]
        .into_iter()
        .flat_map(Stacks::walk)
    }

    /// How many rows are worth the viewer opening now: rows that would be in
    /// the review section on their own — asked of the viewer, asked of nobody,
    /// or a re-review — whose [`Placed::reviewability`] is above
    /// [`Reviewability::CiRunning`].
    ///
    /// Counted by row, not by tree, so a row stacked into another section
    /// still counts where it belongs.
    #[must_use]
    pub fn reviewable_now(&self) -> usize {
        self.walk()
            .filter(|placed| {
                matches!(
                    placed.stack.own,
                    Section::NeedsYourReview | Section::Unclaimed
                )
            })
            .filter(|placed| placed.reviewability() < Reviewability::CiRunning)
            .count()
    }

    /// How many rows the viewer has already reviewed at their head, wherever
    /// they are drawn.
    ///
    /// A review the author has pushed past is not counted: that row is a
    /// re-review, back in [`Board::needs_your_review`].
    #[must_use]
    pub fn you_have_reviewed(&self) -> usize {
        self.walk()
            .filter(|placed| placed.stack.own == Section::YouReviewed)
            .count()
    }

    /// How many pull requests the board was built from.
    ///
    /// Equal to the number of rows handed to [`Board::assemble`], because the
    /// six groups are a partition of them.
    #[must_use]
    pub fn total(&self) -> usize {
        self.walk().count()
    }
}

/// The pull request each row is stacked on, by index, or `None` for a root.
///
/// A row sits on the one open pull request whose head is its base. None, more
/// than one, or itself all make it a root: guessing between two branches that
/// share a name would draw a stack that does not exist. Any row whose chain of
/// bases loops back to itself is a root too, so building the trees ends. A
/// pull request aimed at its own branch is a loop of one and is cut with the
/// rest; this is the only place that protection lives.
///
/// Only a pull request whose head is in this repository can be beneath
/// another. A fork's branch shares nothing with a branch here but its name,
/// and a fork whose head is `main` would otherwise sit under everything aimed
/// at `main`; a head GitHub did not place is treated the same way. Being *on*
/// something asks nothing of the head, since a fork's base is always here.
fn parents(rows: &[BoardRow]) -> Vec<Option<usize>> {
    let mut by_head: HashMap<&BranchName, Vec<usize>> = HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        match row.facts.head_repository {
            HeadRepository::ThisOne => by_head.entry(&row.facts.head_ref).or_default().push(index),
            HeadRepository::Fork | HeadRepository::Unstated => {}
        }
    }
    let mut parents: Vec<Option<usize>> = rows
        .iter()
        .map(
            |row| match by_head.get(&row.facts.base_ref).map(Vec::as_slice) {
                Some(&[only]) => Some(only),
                Some(_) | None => None,
            },
        )
        .collect();

    let looping: Vec<bool> = (0..parents.len())
        .map(|start| {
            let mut at = parents.get(start).copied().flatten();
            for _ in 0..parents.len() {
                match at {
                    Some(next) if next == start => return true,
                    Some(next) => at = parents.get(next).copied().flatten(),
                    None => return false,
                }
            }
            false
        })
        .collect();
    for (parent, looped) in parents.iter_mut().zip(looping) {
        if looped {
            *parent = None;
        }
    }
    parents
}

/// The tree rooted at `at`, taking each row out of `slots` as it is placed.
fn grow(
    at: usize,
    slots: &mut [Option<BoardRow>],
    children: &[Vec<usize>],
    viewer: &Login,
) -> Option<Stack> {
    let row = slots.get_mut(at)?.take()?;
    let mut above: Vec<Stack> = children
        .get(at)
        .into_iter()
        .flatten()
        .filter_map(|&child| grow(child, slots, children, viewer))
        .collect();
    above.sort_by_key(|stack| stack.row.facts.number.get());
    let own = Section::of(&row.facts, viewer);
    // Only where the row is the viewer's to review: a stale review on a draft
    // or on the viewer's own pull request is not a request to look again.
    let rereview = match own {
        Section::NeedsYourReview => row.facts.rereview_due(viewer),
        Section::Yours
        | Section::YourDrafts
        | Section::OtherDrafts
        | Section::Unclaimed
        | Section::YouReviewed => None,
    };
    let reason = match rereview {
        Some(_) => row.reason_once_answered(viewer),
        None => row.reason.clone(),
    };
    Some(Stack {
        row,
        own,
        rereview,
        reason,
        above,
    })
}

#[cfg(test)]
mod tests {
    use super::{Board, BoardRow, Reviewability, Stacks};
    use crate::checks::{Check, CheckState};
    use crate::facts::{
        BranchName, DraftState, HeadRepository, MergeState, MergeableState, PrFacts, PrTitle,
        PrUrl, Review, ReviewDecision, Reviewer, TeamSlug, Verdict,
    };
    use crate::policy::{BlockingChecks, PolicyRules, Requirement};
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

    /// `open_pr`, on `head` in this repository and aimed at `base`.
    fn on_branch(
        number: u32,
        author: &str,
        head: &str,
        base: &str,
    ) -> Result<PrFacts, InvalidValue> {
        let mut pr = open_pr(number, author)?;
        pr.head_ref = BranchName::new(head);
        pr.base_ref = BranchName::new(base);
        Ok(pr)
    }

    /// `on_branch` by `hubot`, with its head in `repository` instead.
    fn on_branch_in(
        repository: HeadRepository,
        number: u32,
        head: &str,
        base: &str,
    ) -> Result<PrFacts, InvalidValue> {
        let mut pr = on_branch(number, "hubot", head, base)?;
        pr.head_repository = repository;
        Ok(pr)
    }

    /// The pull request numbers a section holds, in the order it holds them.
    fn numbers(group: &Stacks) -> Vec<u32> {
        group.rows().map(|row| row.facts().number.get()).collect()
    }

    /// Every row as (number, depth, stacked on), in drawing order.
    fn shape(group: &Stacks) -> Vec<(u32, usize, Option<u32>)> {
        group
            .walk()
            .into_iter()
            .map(|at| {
                (
                    at.row().facts().number.get(),
                    at.depth(),
                    at.stacked_on().map(PrNumber::get),
                )
            })
            .collect()
    }

    /// Reduces every pull request against the same rule and arranges the board.
    fn board_for(prs: Vec<PrFacts>, viewer: &Login) -> Board {
        board_under(&PolicyRules::unprotected(), prs, viewer)
    }

    /// A rule naming one required check, so anything else that fails is
    /// advisory. Needed wherever the blocking-versus-advisory split is what is
    /// under test.
    fn rule_requiring_the_gate() -> Result<PolicyRules, InvalidValue> {
        Ok(PolicyRules {
            blocking_checks: BlockingChecks::from_rule(
                Requirement::Required,
                vec![CheckName::parse("Preview CI Gate")?],
            ),
            ..PolicyRules::unprotected()
        })
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

    /// The commit the viewer reviewed, in the re-review tests: not the head.
    const EARLIER: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    /// The head the author pushed afterwards, in the re-review tests.
    const PUSHED: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    /// `open_pr` by `monalisa` with its head at [`PUSHED`], green under the
    /// gate, and one review by `by` on `on` (`None`: GitHub did not say).
    ///
    /// A change request is also entered where GitHub keeps it — in
    /// `changes_requested_by` and the review decision — because that is the
    /// stale fact a re-review has to see past.
    fn reviewed(
        number: u32,
        by: &Login,
        verdict: Verdict,
        on: Option<&str>,
    ) -> Result<PrFacts, InvalidValue> {
        let mut pr = open_pr(number, "monalisa")?;
        pr.head = HeadSha::parse(PUSHED)?;
        pr.checks = vec![check("Preview CI Gate", "SUCCESS")?];
        pr.reviews = vec![Review {
            by: by.clone(),
            verdict,
            on: on.map(HeadSha::parse).transpose()?,
        }];
        match verdict {
            Verdict::ChangesRequested => {
                pr.changes_requested_by = vec![by.clone()];
                pr.review_decision = Some(ReviewDecision::ChangesRequested);
            }
            Verdict::Approved => pr.review_decision = Some(ReviewDecision::Approved),
        }
        Ok(pr)
    }

    /// Where the one pull request on `board` was drawn, and how it reads:
    /// (needs your review, you reviewed, re-review, placed reviewability,
    /// reviewable now, you have reviewed).
    type Reading = (
        Vec<u32>,
        Vec<u32>,
        Option<Verdict>,
        Option<Reviewability>,
        usize,
        usize,
    );

    fn read(board: &Board) -> Reading {
        let placed = board.walk().next();
        (
            numbers(board.needs_your_review()),
            numbers(board.you_reviewed()),
            placed.and_then(|at| at.rereview()),
            placed.map(|at| at.reviewability()),
            board.reviewable_now(),
            board.you_have_reviewed(),
        )
    }

    #[test]
    fn a_change_request_answered_by_a_push_is_back_in_your_review_list() {
        // GitHub still says CHANGES_REQUESTED, by the viewer. Read naively,
        // the row is waiting on its author; it is waiting on the viewer, who
        // asked on a commit that is no longer the head.
        let reading = Login::parse("octocat").and_then(|me| {
            let pr = reviewed(101, &me, Verdict::ChangesRequested, Some(EARLIER))?;
            Ok::<_, InvalidValue>(read(&board_for(vec![pr], &me)))
        });
        assert_eq!(
            reading,
            Ok((
                vec![101],
                vec![],
                Some(Verdict::ChangesRequested),
                Some(Reviewability::Ready),
                1,
                0
            ))
        );
    }

    #[test]
    fn an_approval_followed_by_a_push_is_back_in_your_review_list() {
        let reading = Login::parse("octocat").and_then(|me| {
            let pr = reviewed(101, &me, Verdict::Approved, Some(EARLIER))?;
            Ok::<_, InvalidValue>(read(&board_for(vec![pr], &me)))
        });
        assert_eq!(
            reading,
            Ok((
                vec![101],
                vec![],
                Some(Verdict::Approved),
                Some(Reviewability::Ready),
                1,
                0
            ))
        );
    }

    #[test]
    fn a_review_on_the_head_stays_where_it_was() {
        let reading = Login::parse("octocat").and_then(|me| {
            let pr = reviewed(101, &me, Verdict::ChangesRequested, Some(PUSHED))?;
            Ok::<_, InvalidValue>(read(&board_for(vec![pr], &me)))
        });
        assert_eq!(
            reading,
            Ok((
                vec![],
                vec![101],
                None,
                Some(Reviewability::ChangesRequested),
                0,
                1
            ))
        );
    }

    #[test]
    fn a_review_github_did_not_place_stays_where_it_was() {
        // No commit is not "some earlier commit": a missing re-review costs
        // less than a false one.
        let reading = Login::parse("octocat").and_then(|me| {
            let pr = reviewed(101, &me, Verdict::ChangesRequested, None)?;
            Ok::<_, InvalidValue>(read(&board_for(vec![pr], &me)))
        });
        assert_eq!(
            reading,
            Ok((
                vec![],
                vec![101],
                None,
                Some(Reviewability::ChangesRequested),
                0,
                1
            ))
        );
    }

    #[test]
    fn somebody_elses_change_request_still_blocks_a_re_review() {
        // Only the viewer's own request is set aside. `hubot` asked too, and
        // nothing says the push answered them, so the author still owes a move
        // and the row is dimmed where it is drawn.
        let reading = Login::parse("octocat").and_then(|me| {
            let mut pr = reviewed(101, &me, Verdict::ChangesRequested, Some(EARLIER))?;
            pr.changes_requested_by.push(Login::parse("hubot")?);
            Ok::<_, InvalidValue>(read(&board_for(vec![pr], &me)))
        });
        assert_eq!(
            reading,
            Ok((
                vec![101],
                vec![],
                Some(Verdict::ChangesRequested),
                Some(Reviewability::ChangesRequested),
                0,
                0
            ))
        );
    }

    #[test]
    fn a_re_review_sorts_as_what_it_is_once_your_request_is_answered() {
        // Read by the row's own reason the re-review is `ChangesRequested`,
        // which sorts below a running build. Once the viewer's answered request
        // is set aside it is green and ready, and comes first. Numbers and
        // pushes both run the other way, so neither tie-break produces this.
        let order = Login::parse("octocat").and_then(|me| {
            let asked = vec![Reviewer::User(me.clone())];

            let mut rereview = reviewed(103, &me, Verdict::ChangesRequested, Some(EARLIER))?;
            rereview.last_push = at(20);

            let mut building = open_pr(102, "hubot")?;
            building.requested_reviewers = asked.clone();
            building.checks = vec![check("Preview CI Gate", "IN_PROGRESS")?];
            building.last_push = at(10);

            let mut failing = open_pr(101, "hubot")?;
            failing.requested_reviewers = asked;
            failing.checks = vec![check("Preview CI Gate", "FAILURE")?];
            failing.last_push = at(0);

            let board = board_for(vec![failing, building, rereview], &me);
            Ok::<_, InvalidValue>(numbers(board.needs_your_review()))
        });
        assert_eq!(order, Ok(vec![103, 102, 101]));
    }

    #[test]
    fn a_pull_request_on_nothing_is_a_root_at_depth_zero() {
        let placed = Login::parse("octocat").and_then(|me| {
            let board = board_for(vec![open_pr(101, "hubot")?], &me);
            Ok::<_, InvalidValue>(
                board
                    .walk()
                    .map(|at| (at.row().facts().number.get(), at.depth(), at.stacked_on()))
                    .collect::<Vec<_>>(),
            )
        });
        assert_eq!(placed, Ok(vec![(101, 0, None)]));
    }

    #[test]
    fn a_stack_is_one_tree_in_order_from_the_bottom() {
        // Handed in top first, so the order is the board's work, not the input's.
        let drawn = Login::parse("octocat").and_then(|me| {
            let board = board_for(
                vec![
                    on_branch(103, "hubot", "stack/three", "stack/two")?,
                    on_branch(101, "hubot", "stack/one", "main")?,
                    on_branch(102, "hubot", "stack/two", "stack/one")?,
                ],
                &me,
            );
            Ok::<_, InvalidValue>(shape(board.unclaimed()))
        });
        assert_eq!(
            drawn,
            Ok(vec![
                (101, 0, None),
                (102, 1, Some(101)),
                (103, 2, Some(102))
            ])
        );
    }

    #[test]
    fn a_base_that_cannot_be_named_with_certainty_makes_a_root() {
        // A merged base, two open pull requests sharing a head, and two loops:
        // in each the honest answer is "this sits on nothing we can show".
        let cases = [
            ("base already merged", vec![(102, "stack/two", "stack/one")]),
            (
                "aimed at its own branch",
                vec![(101, "stack/one", "stack/one")],
            ),
            (
                "two pull requests share the base's name",
                vec![
                    (101, "stack/one", "main"),
                    (104, "stack/one", "main"),
                    (102, "stack/two", "stack/one"),
                ],
            ),
            (
                "a loop",
                vec![
                    (101, "stack/one", "stack/two"),
                    (102, "stack/two", "stack/one"),
                ],
            ),
        ];
        for (what, prs) in cases {
            let drawn = Login::parse("octocat").and_then(|me| {
                let built = prs
                    .iter()
                    .map(|&(n, head, base)| on_branch(n, "hubot", head, base))
                    .collect::<Result<Vec<_>, _>>()?;
                let board = board_for(built, &me);
                Ok::<_, InvalidValue>(shape(board.unclaimed()))
            });
            // Every row present *and* a root. Asking only that the rows found
            // are roots passes when a loop drops them all, because no row in a
            // loop is a root to grow a tree from.
            let mut roots: Vec<(u32, usize, Option<u32>)> =
                prs.iter().map(|&(n, _, _)| (n, 0, None)).collect();
            roots.sort_unstable();
            assert_eq!(drawn, Ok(roots), "{what}: every row must be a root");
        }
    }

    #[test]
    fn a_stack_is_drawn_in_its_roots_section_and_counted_where_each_row_belongs() {
        // The bottom was reviewed by the viewer; the two above it were not.
        let placed = Login::parse("octocat").and_then(|me| {
            let mut bottom = on_branch(101, "hubot", "stack/one", "main")?;
            bottom.reviews = vec![Review {
                by: me.clone(),
                verdict: Verdict::Approved,
                on: None,
            }];
            let board = board_for(
                vec![
                    bottom,
                    on_branch(102, "hubot", "stack/two", "stack/one")?,
                    on_branch(103, "hubot", "stack/three", "stack/two")?,
                ],
                &me,
            );
            Ok::<_, InvalidValue>((
                numbers(board.you_reviewed()),
                numbers(board.unclaimed()),
                board.you_have_reviewed(),
                board.reviewable_now(),
                board.total(),
            ))
        });
        assert_eq!(placed, Ok((vec![101, 102, 103], vec![], 1, 2, 3)));
    }

    #[test]
    fn a_draft_at_the_bottom_takes_its_stack_with_it() {
        let placed = Login::parse("octocat").and_then(|me| {
            let mut bottom = on_branch(101, "hubot", "stack/one", "main")?;
            bottom.draft_state = DraftState::Draft;
            let board = board_for(
                vec![bottom, on_branch(102, "hubot", "stack/two", "stack/one")?],
                &me,
            );
            Ok::<_, InvalidValue>((numbers(board.other_drafts()), numbers(board.unclaimed())))
        });
        assert_eq!(placed, Ok((vec![101, 102], vec![])));
    }

    #[test]
    fn two_branches_cut_from_one_are_siblings_by_number() {
        let drawn = Login::parse("octocat").and_then(|me| {
            let board = board_for(
                vec![
                    on_branch(105, "hubot", "stack/b", "stack/one")?,
                    on_branch(101, "hubot", "stack/one", "main")?,
                    on_branch(103, "hubot", "stack/a", "stack/one")?,
                ],
                &me,
            );
            Ok::<_, InvalidValue>(shape(board.unclaimed()))
        });
        assert_eq!(
            drawn,
            Ok(vec![
                (101, 0, None),
                (103, 1, Some(101)),
                (105, 1, Some(101))
            ])
        );
    }

    /// The unclaimed group's shape, built from `prs`.
    fn unclaimed_shape(
        prs: Vec<Result<PrFacts, InvalidValue>>,
    ) -> Result<Vec<(u32, usize, Option<u32>)>, InvalidValue> {
        let me = Login::parse("octocat")?;
        let built = prs.into_iter().collect::<Result<Vec<_>, _>>()?;
        Ok(shape(board_for(built, &me).unclaimed()))
    }

    #[test]
    fn a_fork_named_like_a_branch_here_is_never_beneath_anything() {
        // A fork's `main` is not this repository's `main`. Read by name, 102
        // would sit on 101, and so would every pull request aimed at `main`.
        let drawn = unclaimed_shape(vec![
            on_branch_in(HeadRepository::Fork, 101, "main", "main"),
            on_branch_in(HeadRepository::ThisOne, 102, "stack/one", "main"),
            on_branch_in(HeadRepository::ThisOne, 103, "stack/two", "stack/one"),
        ]);
        assert_eq!(
            drawn,
            Ok(vec![(101, 0, None), (102, 0, None), (103, 1, Some(102))])
        );
    }

    #[test]
    fn a_head_github_did_not_place_is_never_beneath_anything() {
        // Without GitHub saying the branch is here, `stack/one` may be a fork's
        // branch of that name, and a stack drawn onto it would be invented.
        let drawn = unclaimed_shape(vec![
            on_branch_in(HeadRepository::Unstated, 101, "stack/one", "main"),
            on_branch_in(HeadRepository::ThisOne, 102, "stack/x", "main"),
            on_branch_in(HeadRepository::ThisOne, 103, "stack/two", "stack/one"),
        ]);
        assert_eq!(
            drawn,
            Ok(vec![(101, 0, None), (102, 0, None), (103, 0, None)])
        );
    }

    #[test]
    fn a_fork_can_still_sit_on_a_branch_here() {
        // A fork's base is always in this repository, so only being *beneath*
        // something needs the head to be here.
        let drawn = unclaimed_shape(vec![
            on_branch_in(HeadRepository::ThisOne, 101, "stack/one", "main"),
            on_branch_in(HeadRepository::Fork, 102, "patch-1", "stack/one"),
        ]);
        assert_eq!(drawn, Ok(vec![(101, 0, None), (102, 1, Some(101))]));
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
                // Unclaimed, not `you_reviewed`: nobody asked the viewer
                // about these and the viewer has not reviewed them, so they are
                // the group somebody could usefully pick up.
                numbers(board.unclaimed()),
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
    fn fully_green_outranks_green_enough_to_merge() {
        // Both are readable and neither is blocked, so both used to be `Ready`
        // and the tie fell to push time. But one has something red beside it
        // that somebody may want to look at, and the other has nothing --
        // which makes the second the better use of the next five minutes.
        let order = Login::parse("octocat").and_then(|me| {
            let rule = rule_requiring_the_gate()?;

            let mut spotless = open_pr(1, "hubot")?;
            spotless.requested_reviewers = vec![Reviewer::User(Login::parse("octocat")?)];
            spotless.checks = vec![Check::new(
                CheckName::parse("Preview CI Gate")?,
                CheckState::parse("SUCCESS"),
                Some(at(0)),
            )];
            // *Newer*, so the push tie-break would put it second. Only the
            // rung can put it first, which is the whole point -- with both on
            // the same rung this fixture answers the same either way, and the
            // test proves nothing. It did, until a mutation said so.
            spotless.last_push = at(100);

            let mut advisory = open_pr(2, "hubot")?;
            advisory.requested_reviewers = vec![Reviewer::User(Login::parse("octocat")?)];
            advisory.checks = vec![
                Check::new(
                    CheckName::parse("Preview CI Gate")?,
                    CheckState::parse("SUCCESS"),
                    Some(at(0)),
                ),
                Check::new(
                    CheckName::parse("Not required at all")?,
                    CheckState::parse("FAILURE"),
                    Some(at(0)),
                ),
            ];
            advisory.last_push = at(0);

            let board = board_under(&rule, vec![advisory, spotless], &me);
            Ok::<_, InvalidValue>(numbers(board.needs_your_review()))
        });

        assert_eq!(
            order,
            Ok(vec![1, 2]),
            "nothing failing beats nothing blocking, whatever the push times say"
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
                .unclaimed()
                .rows()
                .flat_map(|row| row.facts().requested_reviewers.iter())
                .filter_map(Reviewer::name)
                .map(str::to_owned)
                .collect::<Vec<_>>();

            Ok::<_, InvalidValue>((
                numbers(board.needs_your_review()),
                numbers(board.unclaimed()),
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
