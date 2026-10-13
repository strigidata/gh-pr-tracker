//! What is true about one pull request, as GitHub reported it.

use chrono::{DateTime, Utc};

use crate::checks::Check;
use crate::types::{wire_value, HeadSha, Login, PrNumber, RepoRef};

/// Strips the padding off a text value GitHub sent.
///
/// The four text newtypes below share this so they cannot drift apart in what
/// counts as whitespace, the same way [`Login`] and [`CheckName`] share theirs.
/// Unlike those, nothing further is enforced — see [`PrTitle`] for why a blank
/// is accepted rather than rejected.
///
/// [`Login`]: crate::Login
/// [`CheckName`]: crate::CheckName
fn trimmed(raw: &str) -> String {
    raw.trim().to_owned()
}

/// The one-line summary an author gave a pull request.
///
/// No decision reads this. It exists for the interruption: a notification
/// saying `#4225` is ready to merge tells nobody what `#4225` is, and the
/// interruption is the product. The board's table does not render it.
///
/// # Why a blank is accepted — do not "harden" this
///
/// The constructor cannot fail, and this is deliberate for all four text values
/// in this module ([`PrUrl`], [`BranchName`], [`TeamSlug`]). It looks like a
/// missing guard and is not one, so the finding behind it is recorded here
/// rather than left to be rediscovered.
///
/// **The state is unreachable.** All four arrive from GraphQL fields the schema
/// declares non-null — `title`, `baseRefName` and `Team.slug` are `String!`,
/// and `url` is `URI!` — so a blank cannot come from a well-formed response at
/// all. A fallible constructor would be a guard against a state the wire
/// cannot produce.
///
/// **And its failure mode is worse than the state it guards.** Rejecting a
/// blank drops the whole pull request off the board, which understates what is
/// holding the team up. Keeping it renders one empty cell. The board is allowed
/// to look incomplete; it is not allowed to go quiet about a blocked pull
/// request.
///
/// So the answer here is neither a smart constructor nor a validation error.
/// Where a value genuinely must not be blank — a [`Login`], a [`CheckName`] —
/// the checked constructors in this crate already say so, and the difference is
/// that those feed decisions rather than cells.
///
/// What the type *does* close is the swap: a title and a URL are both text, and
/// [`PrFacts`] holds them side by side. Two distinct types cannot be
/// transposed at a construction site, however alike they look on the wire:
///
/// ```
/// use prtrack_core::{PrTitle, PrUrl};
///
/// fn announce(_: &PrTitle) {}
///
/// announce(&PrTitle::new("Fix the flaky gate"));
/// let _url = PrUrl::new("https://github.com/octocat/hello-world/pull/4225");
/// ```
///
/// Handing the same function a URL does not compile. Only that one argument
/// differs between the blocks, and every name the failing block needs appears
/// in the passing one, so a renamed constructor reddens the block above rather
/// than leaving this one green for a reason that has nothing to do with types:
///
/// ```compile_fail,E0308
/// use prtrack_core::{PrTitle, PrUrl};
///
/// fn announce(_: &PrTitle) {}
///
/// announce(&PrUrl::new("https://github.com/octocat/hello-world/pull/4225"));
/// ```
///
/// [`CheckName`]: crate::CheckName
/// [`Login`]: crate::Login
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PrTitle(String);

impl PrTitle {
    /// Reads a title from raw text, trimming whitespace around it.
    ///
    /// Never fails, including on a blank — see the type's documentation.
    ///
    /// # Examples
    ///
    /// ```
    /// use prtrack_core::PrTitle;
    ///
    /// assert_eq!(PrTitle::new("  Fix the flaky gate\n").as_str(), "Fix the flaky gate");
    /// ```
    #[must_use]
    pub fn new(raw: &str) -> Self {
        Self(trimmed(raw))
    }

    /// The trimmed title, for the notification that names the pull request.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Where the pull request lives, as GitHub's `url` field gave it.
///
/// This is what a notification opens when someone acts on it, which is the
/// whole loop this tool is built around: a nudge nobody can click is noise.
///
/// It is carried rather than derived because this crate cannot derive it. A
/// [`RepoRef`] and a [`PrNumber`] compose into a `github.com` URL and nothing
/// else, and a GitHub Enterprise host is exactly the deployment where guessing
/// would send a developer to a page that does not exist. The host is the API
/// layer's knowledge, so the API layer's answer is the one kept.
///
/// No syntax is checked. A blank is accepted for the reason given on
/// [`PrTitle`], and beyond that this crate has no business deciding what a
/// valid URL looks like on someone else's host.
///
/// [`PrNumber`]: crate::PrNumber
/// [`RepoRef`]: crate::RepoRef
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PrUrl(String);

impl PrUrl {
    /// Reads a URL from raw text, trimming whitespace around it.
    ///
    /// Never fails — see [`PrTitle`] for why.
    ///
    /// # Examples
    ///
    /// ```
    /// use prtrack_core::PrUrl;
    ///
    /// let url = PrUrl::new("https://github.com/octocat/hello-world/pull/4225");
    /// assert!(url.as_str().ends_with("/pull/4225"));
    /// ```
    #[must_use]
    pub fn new(raw: &str) -> Self {
        Self(trimmed(raw))
    }

    /// The trimmed URL, for the notification's click-through.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The branch a pull request wants to merge into.
///
/// This is the fact that decides whether a branch protection rule governs the
/// pull request at all. A rule protects a pattern of branch names, and a
/// stacked pull request aimed at another feature branch falls outside every one
/// of them — which is why it is judged against [`PolicyRules::unprotected`],
/// where every failing check blocks. Matching this against the rule's pattern
/// is the caller's job; this crate only holds the name.
///
/// # Identity is case-sensitive
///
/// Unlike [`Login`] and [`RepoRef`], which fold case because GitHub accounts
/// genuinely are case-insensitive, git refs are not: `main` and `Main` can both
/// exist and are two branches. Folding case here would hand a pull request the
/// policy of a branch it does not target.
///
/// [`Login`]: crate::Login
/// [`PolicyRules::unprotected`]: crate::PolicyRules::unprotected
/// [`RepoRef`]: crate::RepoRef
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BranchName(String);

impl BranchName {
    /// Reads a branch name from raw text, trimming whitespace around it.
    ///
    /// Never fails — see [`PrTitle`] for why. Trimming is safe rather than
    /// lossy here: git's own ref rules forbid whitespace inside a branch name,
    /// so any that arrived came from the quoting, not from the ref.
    ///
    /// # Examples
    ///
    /// ```
    /// use prtrack_core::BranchName;
    ///
    /// assert_eq!(BranchName::new(" main\n").as_str(), "main");
    /// ```
    #[must_use]
    pub fn new(raw: &str) -> Self {
        Self(trimmed(raw))
    }

    /// The trimmed branch name, for matching against a rule's pattern.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The URL-safe handle of a GitHub team — `backend`, `security-reviewers`.
///
/// A team is one of the things a review can be requested from, and it has no
/// login. Reusing [`Login`] for it would mean either dropping the request or
/// inventing an account that does not exist, and both of those turn "a team
/// has been asked to review" into "nobody has been asked" — the exact
/// misreading that produces a false nudge. See [`Reviewer`].
///
/// # Identity is case-sensitive, and that costs nothing
///
/// GitHub generates the slug from the team's name in lowercase, so there is no
/// second spelling to reconcile — unlike a [`Login`], which a human types into
/// arguments and which therefore has to fold case. Nothing in this tool accepts
/// a slug as input.
///
/// A slug is deliberately not interchangeable with a login. The well-typed call
/// compiles:
///
/// ```
/// use prtrack_core::{Login, TeamSlug};
///
/// fn on_call(_: &TeamSlug) {}
///
/// on_call(&TeamSlug::new("security-reviewers"));
/// let _login = Login::parse("security-reviewers").unwrap();
/// ```
///
/// Handing the same function a login does not, and only that argument differs
/// between the two blocks:
///
/// ```compile_fail,E0308
/// use prtrack_core::{Login, TeamSlug};
///
/// fn on_call(_: &TeamSlug) {}
///
/// on_call(&Login::parse("security-reviewers").unwrap());
/// ```
///
/// [`Login`]: crate::Login
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TeamSlug(String);

impl TeamSlug {
    /// Reads a team slug from raw text, trimming whitespace around it.
    ///
    /// Never fails — see [`PrTitle`] for why.
    ///
    /// # Examples
    ///
    /// ```
    /// use prtrack_core::TeamSlug;
    ///
    /// assert_eq!(TeamSlug::new(" backend ").as_str(), "backend");
    /// ```
    #[must_use]
    pub fn new(raw: &str) -> Self {
        Self(trimmed(raw))
    }

    /// The trimmed slug, for naming who a review is waiting on.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Someone, or something, a review has been requested from.
///
/// A review request is not addressed to an account. GitHub can address one to a
/// team, and a team has a [`TeamSlug`] and no login at all — so a `Vec<Login>`
/// would have to either drop the request or fabricate an account, and both of
/// those turn "the security team was asked" into "nobody was asked". That is
/// the last rung of the blocked-reason ladder and the membership test for the
/// review section of the board, so getting it wrong points the interruption at
/// the wrong person twice over.
///
/// Measured on the target repository, all 71 open review requests are users, so
/// this is not a bug being hit today. It is modelled properly anyway because the
/// API returns a union and `requiresCodeOwnerReviews` is on, which means a
/// `CODEOWNERS` team can appear in it at any time without warning.
///
/// # The viewer test reads [`login`], never the rendered name
///
/// Asking "is this review waiting on me" means comparing accounts. Comparing
/// rendered names instead would match a team whose slug happens to read like
/// the viewer's handle:
///
/// ```
/// use prtrack_core::{Login, Reviewer, TeamSlug};
///
/// let me = Login::parse("security-reviewers").unwrap();
/// let requested = [Reviewer::Team(TeamSlug::new("security-reviewers"))];
///
/// assert!(!requested.iter().any(|r| r.login() == Some(&me)));
/// assert_eq!(requested.first().and_then(Reviewer::name), Some("security-reviewers"));
/// ```
///
/// A pending team request still means someone was asked, so it counts toward
/// "somebody is assigned" even though the viewer cannot be recognised in it.
/// Whether the viewer is *in* that team is a question this crate has no data to
/// answer.
///
/// [`login`]: Reviewer::login
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reviewer {
    /// A person, asked by name.
    User(Login),
    /// A team, asked as a group — usually by `CODEOWNERS`.
    ///
    /// Names no account, so the viewer test cannot recognise the viewer in it
    /// even when they are a member.
    Team(TeamSlug),
    /// A placeholder account for an author imported from another forge, which
    /// GitHub calls a mannequin. It carries a login and behaves as one here.
    Mannequin(Login),
    /// An app acting as a reviewer.
    Bot(Login),

    /// A kind of reviewer this crate does not model, kept as the label the API
    /// gave it.
    ///
    /// GitHub can add members to the union a review request comes from. The
    /// alternative to holding one here is dropping it, which shortens the
    /// requested list and makes a pull request with a reviewer render as having
    /// none — see [`Reviewer::unknown`].
    ///
    /// `#[non_exhaustive]` so that no other crate can build one directly, which
    /// keeps [`Reviewer::unknown`] the single door and the label normalised the
    /// same way every other text in this module is:
    ///
    /// ```compile_fail,E0639
    /// use prtrack_core::Reviewer;
    ///
    /// let _forged = Reviewer::Unknown { kind: "User".to_owned() };
    /// ```
    #[non_exhaustive]
    Unknown {
        /// What the API called this kind of reviewer, trimmed.
        kind: String,
    },
}

impl Reviewer {
    /// Records a review request from a kind of reviewer this crate does not
    /// model.
    ///
    /// `kind` is whatever the API called it. Nothing is derived from the text;
    /// it is kept so that a human debugging an unnamed reviewer can see what
    /// turned up, in the same spirit as [`CheckState::Unknown`].
    ///
    /// # Examples
    ///
    /// ```
    /// use prtrack_core::Reviewer;
    ///
    /// let requested = [Reviewer::unknown("Organization")];
    /// assert_eq!(requested.len(), 1);
    /// assert_eq!(requested.first().and_then(Reviewer::login), None);
    /// ```
    ///
    /// [`CheckState::Unknown`]: crate::CheckState::Unknown
    #[must_use]
    pub fn unknown(kind: &str) -> Self {
        Self::Unknown {
            kind: trimmed(kind),
        }
    }

    /// The account this request is addressed to, if it is addressed to one.
    ///
    /// `None` for a team and for a kind this crate does not model. That is the
    /// honest answer, not an absence to be papered over: those requests are
    /// real and must still be counted, they simply name nobody to compare the
    /// viewer against.
    #[must_use]
    pub const fn login(&self) -> Option<&Login> {
        match self {
            Self::User(login) | Self::Mannequin(login) | Self::Bot(login) => Some(login),
            Self::Team(_) | Self::Unknown { .. } => None,
        }
    }

    /// What to call this reviewer on a board — a handle, or a team's slug.
    ///
    /// `None` where there is nothing to call it, which is a reviewer of a kind
    /// this crate does not model. A renderer should say that a request exists
    /// rather than omit it.
    ///
    /// Never use this for the viewer test; see [`Reviewer::login`].
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::User(login) | Self::Mannequin(login) | Self::Bot(login) => Some(login.as_str()),
            Self::Team(slug) => Some(slug.as_str()),
            Self::Unknown { .. } => None,
        }
    }
}

/// Whether a pull request is asking to be reviewed yet.
///
/// A two-variant enum rather than the `bool` GitHub sends, because this is a
/// category and not a flag: it is the first rung of the blocked-reason ladder
/// and it decides which section of the board a pull request appears in, and
/// both of those read better — and stay exhaustive under `match` — as a named
/// state. The house does the same for [`Requirement`] and
/// [`StaleReviewHandling`].
///
/// [`Requirement`]: crate::Requirement
/// [`StaleReviewHandling`]: crate::StaleReviewHandling
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftState {
    /// The author has not asked for review. Nothing else about the pull
    /// request can make it mergeable, so the ladder stops here.
    Draft,
    /// The pull request is open for review in the ordinary way.
    ReadyForReview,
}

impl From<bool> for DraftState {
    /// Reads GitHub's `isDraft` flag.
    ///
    /// The single place the wire's boolean becomes a state, so the sense of it
    /// is decided and tested once instead of at every boundary that has an
    /// `isDraft` in hand.
    ///
    /// # Examples
    ///
    /// ```
    /// use prtrack_core::DraftState;
    ///
    /// assert_eq!(DraftState::from(true), DraftState::Draft);
    /// ```
    fn from(is_draft: bool) -> Self {
        if is_draft {
            Self::Draft
        } else {
            Self::ReadyForReview
        }
    }
}

/// Where a pull request's head branch lives.
///
/// A branch name alone does not say which repository it is in. A pull request
/// opened from a fork very often has a head called `main`, and read by name
/// alone every pull request aimed at `main` here would look stacked on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadRepository {
    /// This repository: another pull request can be based on its branch.
    ThisOne,
    /// Another repository, almost always a fork. Its branch name says
    /// nothing about any branch here, however alike they look.
    Fork,
    /// GitHub did not say. Treated as "not here" wherever it matters, since
    /// a stack drawn onto a branch that may not exist is a wrong cell.
    Unstated,
}

/// Where GitHub's own review requirements stand on a pull request.
///
/// This is `reviewDecision`, GitHub's summary of the review state *after* it
/// has applied the branch protection rule — how many approvals were needed,
/// whether a code owner had to be among them, whether a push dismissed them.
/// The board takes it as authoritative rather than counting approvals itself,
/// which is why [`PolicyRules::required_approvals`] has no counterpart in
/// [`PrFacts`]: GitHub has already folded the count into this one value.
///
/// Consumed by the ready-to-merge and changes-requested rungs of the
/// blocked-reason ladder.
///
/// # Absence is a state, and it is not [`ReviewRequired`]
///
/// GitHub returns null here when no review is required at all, so the caller
/// holds this as an `Option`. `None` means the repository asks for no reviews;
/// [`ReviewRequired`] means it asks and has not got them. Collapsing the two
/// would report an unreviewable pull request as waiting for a review that is
/// never coming.
///
/// # An unrecognised value stays unrecognised
///
/// Like [`CheckState`], the enum itself is not `#[non_exhaustive]`, so a match
/// over it stays exhaustive without a wildcard arm and a variant added here
/// later breaks every consumer that has to decide what the new value means.
/// Only [`Unknown`] is sealed, and only against construction:
///
/// ```
/// use prtrack_core::ReviewDecision;
///
/// let decision = ReviewDecision::parse("SOMETHING_NEW");
/// assert!(matches!(decision, ReviewDecision::Unknown { .. }));
/// ```
///
/// Forging one outside this crate does not compile, so no caller can label a
/// decision unknown that [`ReviewDecision::parse`] would have recognised —
/// which would hide a request for changes behind a value that merely looks
/// unread. Only the middle line differs from the block above:
///
/// ```compile_fail,E0639
/// use prtrack_core::ReviewDecision;
///
/// let decision = ReviewDecision::Unknown { reported: "APPROVED".to_owned() };
/// assert!(matches!(decision, ReviewDecision::Unknown { .. }));
/// ```
///
/// [`CheckState`]: crate::CheckState
/// [`PolicyRules::required_approvals`]: crate::PolicyRules::required_approvals
/// [`ReviewRequired`]: ReviewDecision::ReviewRequired
/// [`Unknown`]: ReviewDecision::Unknown
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewDecision {
    /// `APPROVED` — the rule's review requirement is satisfied.
    Approved,
    /// `CHANGES_REQUESTED` — a reviewer is holding the pull request back.
    ///
    /// Says *that* someone is; [`PrFacts::changes_requested_by`] says who.
    ChangesRequested,
    /// `REVIEW_REQUIRED` — reviews are required and have not been given.
    ReviewRequired,

    /// A decision no variant above names, kept as the text it arrived as.
    ///
    /// `#[non_exhaustive]` so that no other crate can build one. Reading it
    /// stays open, which is what lets a renderer show the value GitHub sent;
    /// forging one does not.
    #[non_exhaustive]
    Unknown {
        /// The decision exactly as it was reported, trimmed.
        reported: String,
    },
}

impl ReviewDecision {
    /// Reads a review decision from the text GitHub reported.
    ///
    /// Never fails, for the reason given on [`ReviewDecision`]: a value this
    /// crate cannot interpret must still reach the board rather than be dropped
    /// or guessed at.
    ///
    /// Whitespace is stripped and ASCII case folded. Null is *not* this
    /// function's business — the caller holds absence as `None` and only calls
    /// here with a value GitHub actually sent.
    ///
    /// # Examples
    ///
    /// ```
    /// use prtrack_core::ReviewDecision;
    ///
    /// assert_eq!(ReviewDecision::parse(" approved\n"), ReviewDecision::Approved);
    /// assert!(matches!(ReviewDecision::parse(""), ReviewDecision::Unknown { .. }));
    /// ```
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        let (reported, key) = wire_value(raw);
        match key.as_str() {
            "APPROVED" => Self::Approved,
            "CHANGES_REQUESTED" => Self::ChangesRequested,
            "REVIEW_REQUIRED" => Self::ReviewRequired,
            _ => Self::Unknown {
                reported: reported.to_owned(),
            },
        }
    }
}

/// Whether the branch can be merged into its base at all — the git question,
/// not the policy one.
///
/// This is `mergeable`. It answers only "would this merge produce a conflict",
/// which is why it is separate from [`MergeState`]: a pull request can be
/// perfectly mergeable in git terms and still be held back by every rule the
/// repository has. Consumed by the conflict rungs of the ladder, including the
/// approved-but-conflicting case that the board exists to catch — work that
/// passed review and then quietly rotted.
///
/// See [`ReviewDecision`] for how [`Unknown`] is sealed; the same applies here.
///
/// [`Unknown`]: MergeableState::Unknown
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeableState {
    /// `MERGEABLE` — git can merge this branch into its base.
    Mergeable,
    /// `CONFLICTING` — the branch and its base have diverged in the same
    /// places, so someone must rebase or merge before anything else matters.
    Conflicting,
    /// GitHub has not finished working out which of the two it is.
    ///
    /// **GitHub spells this `UNKNOWN`.** It is renamed here because the
    /// catch-all in every enum of this crate is called [`Unknown`], and an enum
    /// with two members meaning different kinds of unknown is a bug waiting for
    /// a `match` arm. This one is not a failure to read the value: mergeability
    /// is computed in the background and the answer is simply not ready, so the
    /// honest rendering is "computing" and the honest next step is to ask
    /// again.
    ///
    /// [`Unknown`]: MergeableState::Unknown
    Computing,

    /// A value no variant above names, kept as the text it arrived as.
    ///
    /// `#[non_exhaustive]` so that no other crate can build one.
    #[non_exhaustive]
    Unknown {
        /// The value exactly as it was reported, trimmed.
        reported: String,
    },
}

impl MergeableState {
    /// Reads a mergeability from the text GitHub reported.
    ///
    /// Never fails; whitespace is stripped and ASCII case folded. Note that
    /// GitHub's `UNKNOWN` parses to [`MergeableState::Computing`], not to
    /// [`MergeableState::Unknown`] — see the variant's own documentation.
    ///
    /// # Examples
    ///
    /// ```
    /// use prtrack_core::MergeableState;
    ///
    /// assert_eq!(MergeableState::parse("CONFLICTING"), MergeableState::Conflicting);
    /// assert_eq!(MergeableState::parse("UNKNOWN"), MergeableState::Computing);
    /// assert!(matches!(MergeableState::parse("SOMETHING_NEW"), MergeableState::Unknown { .. }));
    /// ```
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        let (reported, key) = wire_value(raw);
        match key.as_str() {
            "MERGEABLE" => Self::Mergeable,
            "CONFLICTING" => Self::Conflicting,
            "UNKNOWN" => Self::Computing,
            _ => Self::Unknown {
                reported: reported.to_owned(),
            },
        }
    }
}

/// What the merge button would say — git, checks, reviews, and rules combined.
///
/// This is `mergeStateStatus`, GitHub's own single-word verdict on the whole
/// pull request. The board does not take it as the answer, because it does not
/// say *why*, and the why is the product: `BLOCKED` covers a missing approval,
/// an unresolved thread, and a failing required check alike. What it is uniquely
/// good for is [`Behind`], which no other fact in [`PrFacts`] reports — a branch
/// that is merely out of date rather than in conflict.
///
/// See [`ReviewDecision`] for how [`Unknown`] is sealed; the same applies here.
///
/// [`Behind`]: MergeState::Behind
/// [`Unknown`]: MergeState::Unknown
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeState {
    /// `BEHIND` — the base branch has moved on and the rule requires being up
    /// to date. Only blocking where [`PolicyRules::up_to_date_branch`] says so.
    ///
    /// [`PolicyRules::up_to_date_branch`]: crate::PolicyRules::up_to_date_branch
    Behind,
    /// `BLOCKED` — a rule is holding the merge. Which rule is not said here;
    /// the rest of [`PrFacts`] is what answers that.
    Blocked,
    /// `CLEAN` — nothing GitHub knows of is in the way.
    Clean,
    /// `DIRTY` — the merge would conflict.
    ///
    /// Overlaps [`MergeableState::Conflicting`] and is reported separately, so
    /// the two can disagree — most often while [`MergeableState::Computing`] is
    /// still outstanding and one of the pair has been recomputed and the other
    /// has not. Anything deciding a conflict rung should pick one of them as
    /// its authority and say which, rather than reading whichever comes to
    /// hand; [`mergeable`] is the narrower question and the better default.
    ///
    /// [`mergeable`]: PrFacts::mergeable
    Dirty,
    /// `DRAFT` — the pull request is a draft, so it cannot merge whatever else
    /// is true. Reported independently of [`PrFacts::draft_state`], and the two
    /// can disagree if the payload was assembled from separate reads.
    Draft,
    /// `HAS_HOOKS` — mergeable, but a pre-receive hook still gets a say.
    HasHooks,
    /// `UNSTABLE` — mergeable with a non-required check failing. The state that
    /// makes the required-versus-advisory distinction visible on the wire.
    Unstable,
    /// GitHub could not work the state out.
    ///
    /// **GitHub spells this `UNKNOWN`**, renamed for the reason given on
    /// [`MergeableState::Computing`].
    Undetermined,

    /// A value no variant above names, kept as the text it arrived as.
    ///
    /// `#[non_exhaustive]` so that no other crate can build one.
    #[non_exhaustive]
    Unknown {
        /// The value exactly as it was reported, trimmed.
        reported: String,
    },
}

impl MergeState {
    /// Reads a merge state from the text GitHub reported.
    ///
    /// Never fails; whitespace is stripped and ASCII case folded. GitHub's
    /// `UNKNOWN` parses to [`MergeState::Undetermined`], not to
    /// [`MergeState::Unknown`].
    ///
    /// # Examples
    ///
    /// ```
    /// use prtrack_core::MergeState;
    ///
    /// assert_eq!(MergeState::parse("has_hooks"), MergeState::HasHooks);
    /// assert_eq!(MergeState::parse("UNKNOWN"), MergeState::Undetermined);
    /// ```
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        let (reported, key) = wire_value(raw);
        match key.as_str() {
            "BEHIND" => Self::Behind,
            "BLOCKED" => Self::Blocked,
            "CLEAN" => Self::Clean,
            "DIRTY" => Self::Dirty,
            "DRAFT" => Self::Draft,
            "HAS_HOOKS" => Self::HasHooks,
            "UNSTABLE" => Self::Unstable,
            "UNKNOWN" => Self::Undetermined,
            _ => Self::Unknown {
                reported: reported.to_owned(),
            },
        }
    }
}

/// Everything known about one open pull request at one moment.
///
/// This is the input side of every decision the crate makes. It is facts only:
/// nothing here has been judged, because judging needs the repository's
/// [`PolicyRules`], and the same pull request gets different verdicts under
/// different rules. Pairing the two is the caller's move — see [`classify`] for
/// the check half of it.
///
/// Every field earns its place by naming the consumer it is *intended* for. A
/// fact nothing will ever consume is a fact that can go stale unnoticed, which
/// is the failure this crate exists to prevent. Note the tense: several of
/// those consumers — the blocked-reason ladder, the sections, the notifier —
/// are still being written, so for now the field documentation records what
/// each fact is for rather than a call site you can go and read.
///
/// # All the fields are public, and adding one is a breaking change
///
/// There is no constructor. Seventeen facts do not fit in an argument list a
/// caller can get right, and there is no meaningful default to build from the
/// way [`PolicyRules::unprotected`] serves as one — a pull request with no
/// number or no head commit is not a pull request. So a construction site names
/// every field, and the struct is deliberately *not* `#[non_exhaustive]`:
/// when GitHub grows a fact worth acting on, every place that assembles a
/// `PrFacts` should stop compiling until someone decides what to put there.
///
/// # It identifies itself: [`repo`] and [`number`] together
///
/// A [`PrNumber`] is unique only inside its repository, so the two travel
/// together and nothing downstream has to thread the repository alongside as
/// implicit context. That is what a notification's dedupe key is built from,
/// and a key missing the repository merges two different pull requests that
/// happen to share a number — one nudge where two were owed, about work in a
/// repository the reader never sees named.
///
/// [`repo`]: PrFacts::repo
/// [`number`]: PrFacts::number
/// [`PolicyRules`]: crate::PolicyRules
/// [`PolicyRules::unprotected`]: crate::PolicyRules::unprotected
/// [`PrNumber`]: crate::PrNumber
/// [`classify`]: crate::classify
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrFacts {
    /// Which repository this pull request belongs to.
    ///
    /// Carried rather than assumed. It scopes [`number`] into something that
    /// identifies a pull request on its own, which is what a notification
    /// dedupe key and any board spanning more than one repository need. It also
    /// says which branch protection rule this pull request is judged against,
    /// since a policy read from one repository must never be applied to
    /// another's work.
    ///
    /// Identity folds case, because GitHub's does — see [`RepoRef`]. One pull
    /// request restated in another casing is therefore one pull request, not
    /// two nudges for the same work.
    ///
    /// [`number`]: PrFacts::number
    /// [`RepoRef`]: crate::RepoRef
    pub repo: RepoRef,

    /// The number in the URL, and the one people say out loud.
    ///
    /// Every rendering leads with it and every notification names it. With
    /// [`repo`] it is the stable handle for "the same pull request as last
    /// poll", which is what turns a state change into a nudge.
    ///
    /// [`repo`]: PrFacts::repo
    pub number: PrNumber,

    /// Where to go to act on it — the target of the notification's click.
    pub url: PrUrl,

    /// What the pull request is called, for a notification that has to make
    /// sense on a lock screen. The board's table does not render it.
    pub title: PrTitle,

    /// Who opened it, if the account still exists.
    ///
    /// Read to decide whether this is the viewer's own work, which is the
    /// difference between "you are blocking your team" and "your team is
    /// blocking you" — two different sections and two different nudges.
    ///
    /// `None` is a deleted account, which GitHub reports as a null author.
    /// It is not a placeholder to be filled in: a substituted name would
    /// collide with the real account of anyone who registers that handle, and a
    /// `None` author correctly matches no viewer at all.
    pub author: Option<Login>,

    /// Whether the author is asking for review yet.
    ///
    /// The first rung of the blocked-reason ladder, and it outranks every other
    /// reason: a draft cannot merge however green it is, so nothing further
    /// needs saying about it. Also decides which section it lands in.
    pub draft_state: DraftState,

    /// The commit every other fact here is a statement about.
    ///
    /// Reviews, checks, and mergeability are all claims about one commit, and a
    /// push replaces it. A consumer comparing two snapshots uses this to tell a
    /// real change from a stale reading: facts carried over across a change of
    /// head are the fresh-looking stale cell this crate is built to avoid.
    pub head: HeadSha,

    /// The pull request's own branch — what you would check out to work on it,
    /// and the thing a reader copies rather than retypes.
    pub head_ref: BranchName,

    /// Which repository [`head_ref`] is a branch of.
    ///
    /// Only a head in this repository can be the base of another pull request
    /// here, so only such a pull request can have a stack drawn on top of it.
    ///
    /// [`head_ref`]: PrFacts::head_ref
    pub head_repository: HeadRepository,

    /// The branch it wants to merge into.
    ///
    /// Read to decide *which* [`PolicyRules`] apply. A rule protects a pattern
    /// of branch names; a pull request outside every pattern — a stacked one
    /// aimed at another feature branch — is judged against
    /// [`PolicyRules::unprotected`], where nothing is advisory.
    ///
    /// [`PolicyRules`]: crate::PolicyRules
    /// [`PolicyRules::unprotected`]: crate::PolicyRules::unprotected
    pub base_ref: BranchName,

    /// Where GitHub's own review requirement stands, or `None` where the
    /// repository requires no review at all.
    ///
    /// Drives the ready-to-merge rung and, with [`mergeable`], the
    /// approved-but-conflicting one — approved work that rotted, which is the
    /// most expensive thing this board finds.
    ///
    /// [`mergeable`]: PrFacts::mergeable
    pub review_decision: Option<ReviewDecision>,

    /// Whether git can merge the branch at all.
    ///
    /// The conflict rungs read this. It is the git question only; a mergeable
    /// pull request can still be blocked by every rule the repository has.
    pub mergeable: MergeableState,

    /// GitHub's own one-word verdict on the merge button.
    ///
    /// Used for the one thing nothing else here reports: whether the branch is
    /// merely [`Behind`] its base. Not used as the answer, because it does not
    /// say why.
    ///
    /// [`Behind`]: MergeState::Behind
    pub merge_state: MergeState,

    /// Who has been asked to review and has not yet.
    ///
    /// Two consumers: the last rungs of the ladder, which name who is being
    /// waited on and fall through to "nobody assigned" when this is empty; and
    /// the review section, which asks whether the viewer is among them. See
    /// [`Reviewer`] — a request can be addressed to a team, which counts as
    /// somebody being asked while naming no account for the viewer test to
    /// match.
    pub requested_reviewers: Vec<Reviewer>,

    /// The reviewers currently holding the pull request back, from GitHub's
    /// latest review per reviewer, so nobody appears twice.
    ///
    /// This rung outranks a failing required check, because a human waiting on
    /// another human is the more expensive stall. It answers *who*;
    /// [`review_decision`] answers *whether*, and it is the authority of the
    /// two — a review left by an account since deleted still counts against the
    /// pull request but cannot be named here.
    ///
    /// [`review_decision`]: PrFacts::review_decision
    pub changes_requested_by: Vec<Login>,

    /// Everyone whose latest review is on the record, whatever it said.
    ///
    /// Superset of [`changes_requested_by`]: an approval counts too. Read to
    /// answer one question the request list cannot — *have I already looked at
    /// this?* GitHub removes a reviewer from `requested_reviewers` the moment
    /// they submit, so an approved pull request and one nobody was ever asked
    /// about are indistinguishable without this.
    ///
    /// [`changes_requested_by`]: PrFacts::changes_requested_by
    pub reviewed_by: Vec<Login>,

    /// How many review threads are still open.
    ///
    /// Blocking only where [`PolicyRules::conversation_resolution`] says the
    /// repository requires resolution; informational everywhere else. Reading
    /// it without the policy invents a blocker on most repositories.
    ///
    /// [`PolicyRules::conversation_resolution`]: crate::PolicyRules::conversation_resolution
    pub unresolved_threads: u32,

    /// How many comments the pull request has collected.
    ///
    /// Rendered beside [`unresolved_threads`] as a sense of how much discussion
    /// has happened. Never blocks anything.
    ///
    /// [`unresolved_threads`]: PrFacts::unresolved_threads
    pub comments: u32,

    /// The check rollup exactly as reported: every *attempt*, unmerged and
    /// unjudged.
    ///
    /// Hand this to [`classify`] together with the policy. Do not reduce it
    /// first. GitHub leaves a re-run's predecessor in the rollup, so one name
    /// arrives several times with different verdicts, and only the newest is
    /// current — [`classify`] is where that rule lives, and where a check's
    /// failure is weighed against the required list.
    ///
    /// [`classify`]: crate::classify
    pub checks: Vec<Check>,

    /// When the head commit was created, as GitHub's `committedDate`.
    ///
    /// The board's age column, the tie-break for every section's sort, and the
    /// measure a "this has been sitting for days" nudge is taken against — with
    /// `now` supplied by the caller, since this crate never reads a clock.
    ///
    /// It is the commit's date, not the push's, which is the closest thing the
    /// API offers. Rebasing or amending refreshes it; pushing a commit written
    /// last week does not. It is never absent: a pull request has at least one
    /// commit, so a payload without one is a fetch that went wrong and belongs
    /// in an error rather than in a fabricated instant here.
    pub last_push: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::{
        BranchName, DraftState, HeadRepository, MergeState, MergeableState, PrFacts, PrTitle,
        PrUrl, ReviewDecision, Reviewer, TeamSlug,
    };
    use crate::checks::{classify, Check, CheckState};
    use crate::policy::{BlockingChecks, PolicyRules, Requirement};
    use crate::types::{CheckName, HeadSha, InvalidValue, Login, PrNumber, RepoRef};
    use chrono::{DateTime, TimeDelta, Utc};

    /// A named text value's constructor, read back as the string it kept, so
    /// four types that share a normalisation rule can be tested through one
    /// signature.
    type TextValue = (&'static str, fn(&str) -> String);

    /// A named reviewer variant that carries an account, so the three that do
    /// can be tested through one signature.
    type AccountReviewer = (&'static str, fn(Login) -> Reviewer);

    /// Every text value in this module, behind one signature, so the rules they
    /// share are asserted once for all of them rather than four times over.
    /// A type that stopped normalising, or started rejecting, fails here.
    const TEXT_VALUES: [TextValue; 4] = [
        ("PrTitle", |raw| PrTitle::new(raw).as_str().to_owned()),
        ("PrUrl", |raw| PrUrl::new(raw).as_str().to_owned()),
        ("BranchName", |raw| BranchName::new(raw).as_str().to_owned()),
        ("TeamSlug", |raw| TeamSlug::new(raw).as_str().to_owned()),
    ];

    #[test]
    fn a_text_value_keeps_what_it_was_given_once_the_padding_is_off() {
        for (type_name, construct) in TEXT_VALUES {
            for (raw, expected) in [
                ("octocat", "octocat"),
                ("  octocat", "octocat"),
                ("octocat\n", "octocat"),
                (" \tsome value \r\n", "some value"),
                ("inner  spacing kept", "inner  spacing kept"),
            ] {
                assert_eq!(
                    construct(raw),
                    expected,
                    "{type_name} must normalise {raw:?} to the bare value"
                );
            }
        }
    }

    #[test]
    fn a_blank_text_value_is_kept_rather_than_rejected() {
        // Deliberate, and the one illegal state this module leaves
        // representable. GitHub declares all four of these non-null, so a blank
        // means the payload was corrupt — and refusing it would drop the whole
        // pull request off the board, which understates what is blocking the
        // team. An empty cell is the smaller lie. Change this only by making
        // the constructors fallible, never by inventing a placeholder.
        for (type_name, construct) in TEXT_VALUES {
            for raw in ["", " ", "\t\r\n"] {
                assert_eq!(
                    construct(raw),
                    "",
                    "{type_name} must keep {raw:?} as a blank rather than reject it"
                );
            }
        }
    }

    /// `Example-Org/example-repo#3449` as the audit found it: approved, its only
    /// failing checks advisory, conflicting with `main`, and untouched for 21
    /// days. It was invisible until the branch protection rule was read,
    /// because painting every failure red blamed its author for a check the
    /// merge button ignores.
    ///
    /// Built through the real constructors, so a fixture that stopped parsing
    /// cannot leave the assertions below quietly asserting nothing.
    fn pr_3449() -> Result<PrFacts, InvalidValue> {
        Ok(PrFacts {
            repo: RepoRef::parse("Example-Org/example-repo")?,
            number: PrNumber::new(3449)?,
            url: PrUrl::new("https://github.com/Example-Org/example-repo/pull/3449"),
            title: PrTitle::new("Split the ledger writer"),
            author: Some(Login::parse("octocat")?),
            draft_state: DraftState::ReadyForReview,
            head: HeadSha::parse("9f2c4e1b7a3d5068c9e4b2a17f36d8095c1e4a7b")?,
            head_ref: BranchName::new("topic/branch-01"),
            head_repository: HeadRepository::ThisOne,
            base_ref: BranchName::new("main"),
            review_decision: Some(ReviewDecision::Approved),
            mergeable: MergeableState::Conflicting,
            merge_state: MergeState::Dirty,
            requested_reviewers: vec![Reviewer::Team(TeamSlug::new("ledger-owners"))],
            changes_requested_by: Vec::new(),
            reviewed_by: Vec::new(),
            unresolved_threads: 0,
            comments: 4,
            checks: vec![
                // The required gate failed and was re-run green. Only the
                // newest attempt is current.
                Check::new(
                    CheckName::parse("Preview CI Gate")?,
                    CheckState::parse("FAILURE"),
                    Some(DateTime::<Utc>::UNIX_EPOCH),
                ),
                Check::new(
                    CheckName::parse("Preview CI Gate")?,
                    CheckState::parse("SUCCESS"),
                    Some(DateTime::<Utc>::UNIX_EPOCH + TimeDelta::minutes(9)),
                ),
                // Red, and the merge button does not care.
                Check::new(
                    CheckName::parse("codecov/patch")?,
                    CheckState::parse("FAILURE"),
                    Some(DateTime::<Utc>::UNIX_EPOCH),
                ),
            ],
            last_push: DateTime::<Utc>::UNIX_EPOCH,
        })
    }

    #[test]
    fn a_pull_request_is_identified_by_its_repository_and_number_together() {
        // A number is unique only inside its repository, so a dedupe key built
        // on the number alone silently merges `octocat/one#4225` with
        // `octocat/two#4225` — one nudge where two were owed, about work in a
        // repository the reader never sees named.
        let keyed = |repo: &str, number: u32| -> Result<(RepoRef, PrNumber), InvalidValue> {
            let facts = PrFacts {
                repo: RepoRef::parse(repo)?,
                number: PrNumber::new(number)?,
                ..pr_3449()?
            };
            Ok((facts.repo, facts.number))
        };

        let two_repositories = keyed("octocat/one", 4225)
            .and_then(|first| keyed("octocat/two", 4225).map(|second| first == second));
        assert_eq!(
            two_repositories,
            Ok(false),
            "the same number in two repositories is two pull requests"
        );

        // The other half, without which the assertion above would also pass
        // against a key that made every pull request distinct. `RepoRef` folds
        // case the way GitHub does, so a `--repo` typed by hand and the API's
        // canonical casing are one pull request, not two nudges.
        let one_repository_restated = keyed("Example-Org/example-repo", 3449).and_then(|first| {
            keyed("example-org/EXAMPLE-REPO", 3449).map(|second| first == second)
        });
        assert_eq!(
            one_repository_restated,
            Ok(true),
            "one pull request restated in another casing is still one pull request"
        );
    }

    #[test]
    fn the_checks_a_pull_request_carries_are_the_attempts_the_policy_still_has_to_judge() {
        // `checks` holds the rollup as reported — every attempt, unmerged and
        // unjudged — because whether a failure blocks is the repository's call
        // and this bundle does not know the repository's rule. Handing the
        // field straight to `classify` is the whole contract, and it is what
        // separates the one advisory failure from a required check that had
        // already gone green again.
        fn names(checks: &[Check]) -> Vec<String> {
            checks
                .iter()
                .map(|check| check.name().as_str().to_owned())
                .collect()
        }

        // Kept inside the `Result` so that a required context which stopped
        // parsing yields `Err` rather than a rule that quietly requires
        // nothing — under which every assertion below would still pass.
        let verdict = CheckName::parse("Preview CI Gate").and_then(|gate| {
            let rule = PolicyRules {
                blocking_checks: BlockingChecks::from_rule(Requirement::Required, vec![gate]),
                ..PolicyRules::unprotected()
            };
            pr_3449().map(|facts| {
                let classified = classify(&facts.checks, &rule);
                (
                    names(classified.blocking()),
                    names(classified.advisory()),
                    names(classified.passing()),
                )
            })
        });

        assert_eq!(
            verdict,
            Ok((
                Vec::new(),
                vec!["codecov/patch".to_owned()],
                vec!["Preview CI Gate".to_owned()],
            )),
            "nothing required is failing here; the red check is one the merge button ignores"
        );
    }

    #[test]
    fn a_reviewer_who_holds_an_account_names_it() {
        let cases: [AccountReviewer; 3] = [
            ("User", Reviewer::User),
            ("Bot", Reviewer::Bot),
            ("Mannequin", Reviewer::Mannequin),
        ];
        for (kind, build) in cases {
            let reviewer = Login::parse("octocat").map(build);
            assert_eq!(
                reviewer.as_ref().map(|r| r.login().map(Login::as_str)),
                Ok(Some("octocat")),
                "a {kind} review request is addressed to an account"
            );
            assert_eq!(
                reviewer.as_ref().map(Reviewer::name),
                Ok(Some("octocat")),
                "and that account's handle is what a board renders for a {kind}"
            );
        }
    }

    #[test]
    fn a_team_review_request_names_the_team_and_no_account() {
        let team = Reviewer::Team(TeamSlug::new("security-reviewers"));
        assert_eq!(
            team.login(),
            None,
            "a team has no login, and inventing one would address a nudge to an account that does not exist"
        );
        assert_eq!(
            team.name(),
            Some("security-reviewers"),
            "the slug is what a board renders in place of a handle"
        );
    }

    #[test]
    fn a_team_slug_that_reads_like_a_login_is_still_not_that_user() {
        // The reason `Reviewer` is a sum type rather than a `Vec<Login>`. A
        // team and a user can carry the same text, and `security-reviewers` the
        // team requesting a review does not mean `security-reviewers` the
        // account has been asked. Were the slug forced into a `Login`, the
        // viewer test below would fire and this tool would interrupt someone
        // over a review nobody asked them for — the false nudge that gets it
        // uninstalled.
        const SHARED_TEXT: &str = "security-reviewers";

        let asked_the_team = Login::parse(SHARED_TEXT).map(|me| {
            let requested = [Reviewer::Team(TeamSlug::new(SHARED_TEXT))];
            requested.iter().any(|r| r.login() == Some(&me))
        });
        assert_eq!(
            asked_the_team,
            Ok(false),
            "a request to the team must not read as a request to the account"
        );

        // The control, without which the assertion above would also pass
        // against a `login()` that returned `None` for everything.
        let asked_the_user = Login::parse(SHARED_TEXT).map(|me| {
            let requested = [Reviewer::User(me.clone())];
            requested.iter().any(|r| r.login() == Some(&me))
        });
        assert_eq!(
            asked_the_user,
            Ok(true),
            "a request to the account itself must still be found"
        );
    }

    #[test]
    fn a_reviewer_of_a_kind_this_crate_does_not_model_still_counts_as_assigned() {
        // GitHub's requested-reviewer union can grow. Dropping a member it
        // grows into would leave `requested_reviewers` short, and a pull
        // request whose only reviewer went missing renders as "nobody
        // assigned" — a nudge aimed at the wrong person entirely.
        let requested = [Reviewer::unknown("  Organization\n")];
        assert_eq!(
            requested.len(),
            1,
            "the request happened and must be counted"
        );
        assert_eq!(
            requested.first().map(Reviewer::login),
            Some(None),
            "nothing is known to address, so nothing is claimed"
        );
        assert_eq!(
            requested.first().map(Reviewer::name),
            Some(None),
            "and nothing is claimed as a name either"
        );
        assert_eq!(
            requested.first(),
            Some(&Reviewer::unknown("Organization")),
            "the kind survives, trimmed, so a human can see what turned up"
        );
    }

    #[test]
    fn a_branch_name_is_case_sensitive_because_git_refs_are() {
        // `main` and `Main` are two branches, and a rule protecting one does not
        // govern a pull request aimed at the other. Folding case here — as
        // `Login` does, because GitHub accounts really are case-insensitive —
        // would apply a policy that does not apply.
        assert_ne!(BranchName::new("main"), BranchName::new("Main"));
    }

    #[test]
    fn a_draft_flag_reads_the_same_way_every_time_it_is_converted() {
        // The one place the wire's boolean becomes a state. Getting it
        // backwards would file every ready pull request as a draft and hide the
        // whole board, so the mapping is pinned here rather than re-derived at
        // each boundary.
        assert_eq!(DraftState::from(true), DraftState::Draft);
        assert_eq!(DraftState::from(false), DraftState::ReadyForReview);
    }

    #[test]
    fn every_review_decision_github_reports_parses_to_the_variant_that_names_it() {
        let cases: [(&str, ReviewDecision); 3] = [
            ("APPROVED", ReviewDecision::Approved),
            ("CHANGES_REQUESTED", ReviewDecision::ChangesRequested),
            ("REVIEW_REQUIRED", ReviewDecision::ReviewRequired),
        ];
        for (raw, expected) in cases {
            assert_eq!(
                ReviewDecision::parse(raw),
                expected,
                "{raw:?} must parse to the decision that names it"
            );
        }
    }

    #[test]
    fn every_mergeable_state_github_reports_parses_to_the_variant_that_names_it() {
        let cases: [(&str, MergeableState); 3] = [
            ("MERGEABLE", MergeableState::Mergeable),
            ("CONFLICTING", MergeableState::Conflicting),
            ("UNKNOWN", MergeableState::Computing),
        ];
        for (raw, expected) in cases {
            assert_eq!(
                MergeableState::parse(raw),
                expected,
                "{raw:?} must parse to the state that names it"
            );
        }
    }

    #[test]
    fn every_merge_state_github_reports_parses_to_the_variant_that_names_it() {
        let cases: [(&str, MergeState); 8] = [
            ("BEHIND", MergeState::Behind),
            ("BLOCKED", MergeState::Blocked),
            ("CLEAN", MergeState::Clean),
            ("DIRTY", MergeState::Dirty),
            ("DRAFT", MergeState::Draft),
            ("HAS_HOOKS", MergeState::HasHooks),
            ("UNSTABLE", MergeState::Unstable),
            ("UNKNOWN", MergeState::Undetermined),
        ];
        for (raw, expected) in cases {
            assert_eq!(
                MergeState::parse(raw),
                expected,
                "{raw:?} must parse to the state that names it"
            );
        }
    }

    #[test]
    fn a_value_this_crate_does_not_know_keeps_the_text_it_arrived_as() {
        // GitHub adds enum values. Mapping one onto the nearest known variant
        // would turn a state nobody has read into a verdict, so the text
        // survives instead.
        //
        // The padding comes off and nothing else is touched. A probe that is
        // already upper-case and unpadded cannot show that, because it reads
        // the same whether the spelling was preserved or rebuilt from the
        // normalised key — so every row below differs from its normalised form
        // in case, in padding, or in both.
        let unrecognised: [(&str, &str); 3] = [
            ("SOMETHING_NEW", "SOMETHING_NEW"),
            ("  Something_New\n", "Something_New"),
            ("\tsomething_new ", "something_new"),
        ];
        for (raw, kept) in unrecognised {
            assert_eq!(
                ReviewDecision::parse(raw),
                ReviewDecision::Unknown {
                    reported: kept.to_owned()
                },
                "a review decision of {raw:?} must reach a reader as {kept:?}"
            );
            assert_eq!(
                MergeableState::parse(raw),
                MergeableState::Unknown {
                    reported: kept.to_owned()
                },
                "a mergeability of {raw:?} must reach a reader as {kept:?}"
            );
            assert_eq!(
                MergeState::parse(raw),
                MergeState::Unknown {
                    reported: kept.to_owned()
                },
                "a merge state of {raw:?} must reach a reader as {kept:?}"
            );
        }
    }

    #[test]
    fn a_value_is_read_the_same_however_it_is_cased_or_padded() {
        for raw in [
            "CONFLICTING",
            "conflicting",
            "Conflicting",
            "  CONFLICTING ",
            "\tconflicting\r\n",
        ] {
            assert_eq!(
                MergeableState::parse(raw),
                MergeableState::Conflicting,
                "{raw:?} names the same state as CONFLICTING"
            );
        }
    }
}
