//! The repository policy a pull request is judged against.

use crate::types::CheckName;

/// Whether a branch protection rule enforces something.
///
/// Stands in for a `bool` field so that a call site reads as the question it is
/// answering. It is shared across the several independent things a rule can
/// demand, because the answer is the same shape every time — which does mean it
/// guards the reading of a field, not the choice of one: two fields of this
/// type can still be swapped for each other and compile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requirement {
    /// The rule enforces it.
    Required,
    /// The rule does not enforce it, so it cannot hold a merge back.
    NotRequired,
}

/// What a rule does to existing approvals when new commits arrive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StaleReviewHandling {
    /// A push drops the approvals a pull request had already collected.
    DismissOnPush,
    /// Approvals survive a push.
    Keep,
}

/// A non-empty set of the checks a branch protection rule names as required.
///
/// Non-emptiness is the whole point of the type. An empty required list does
/// **not** mean "no check can block a merge" — it means the policy told us
/// nothing, and the safe reading of that is the opposite. [`BlockingChecks`]
/// holds that case in its own variant, so a value of this type can only ever
/// mean "these exact checks and no others".
///
/// It is a set rather than a list: a name the rule happens to list twice is one
/// required check, and counting it twice would overstate what a pull request is
/// waiting on. The order the rule gave is kept, since that is the order a
/// reader recognises.
///
/// # Only [`BlockingChecks::from_rule`] can build one
///
/// The constructor is deliberately private. A required set is only meaningful
/// alongside the rule's `requiresStatusChecks` flag, and a caller able to build
/// one directly could wrap it in [`BlockingChecks::Only`] under a rule that
/// enforces nothing — reporting a check as advisory when the fallback says
/// every failure blocks. Read one out of a [`BlockingChecks::Only`] you were
/// given; build one only by going through the rule.
///
/// # Examples
///
/// ```
/// use prtrack_core::{BlockingChecks, CheckName, Requirement};
///
/// let gate = CheckName::parse("Preview CI Gate").unwrap();
/// let listed_twice = vec![gate.clone(), gate];
///
/// let BlockingChecks::Only { required, .. } =
///     BlockingChecks::from_rule(Requirement::Required, listed_twice)
/// else {
///     panic!("a rule that names a context must not fall back")
/// };
/// assert_eq!(required.as_slice().len(), 1);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequiredChecks(Vec<CheckName>);

impl RequiredChecks {
    /// Collects the checks a rule requires, keeping the first of any repeat.
    ///
    /// Returns `None` for an empty list. That is not an error and should not be
    /// reported as one: a rule naming no checks is a legitimate policy with its
    /// own meaning; [`BlockingChecks::from_rule`] maps it to that meaning.
    ///
    /// Private on purpose — see the type's documentation.
    fn new(checks: Vec<CheckName>) -> Option<Self> {
        let mut unique: Vec<CheckName> = Vec::with_capacity(checks.len());
        for check in checks {
            if !unique.contains(&check) {
                unique.push(check);
            }
        }
        (!unique.is_empty()).then_some(Self(unique))
    }

    /// The required checks, first listed first, for rendering the policy itself.
    #[must_use]
    pub fn as_slice(&self) -> &[CheckName] {
        &self.0
    }
}

/// Which failing checks can hold a merge back.
///
/// The variants are named for their consequence rather than for what the API
/// returned, because the consequence is what call sites get wrong. A rule that
/// names no required checks is not "an empty list"; it is [`Every`], and the
/// only way to ask this type a question is to have said which of the two you
/// are holding.
///
/// # A consumer can read the set but cannot forge one
///
/// Matching stays exhaustive without a wildcard arm, so a variant added here
/// later still breaks every downstream match instead of vanishing into a
/// catch-all:
///
/// ```
/// use prtrack_core::{BlockingChecks, CheckName, Requirement};
///
/// let gate = CheckName::parse("Preview CI Gate").unwrap();
/// let policy = BlockingChecks::from_rule(Requirement::Required, vec![gate]);
///
/// let named = match policy {
///     BlockingChecks::Only { ref required, .. } => required.as_slice().len(),
///     BlockingChecks::Every => 0,
/// };
/// assert_eq!(named, 1);
/// ```
///
/// Building one outside this crate does not compile, so a required set read
/// from one rule cannot be moved onto another that enforces nothing — which
/// would report a blocking check as advisory. Only the last two lines differ
/// from the block above, and every name the failing block needs appears in the
/// passing one, so a renamed constructor reddens that block rather than leaving
/// this one passing for an unrelated reason:
///
/// ```compile_fail,E0639
/// use prtrack_core::{BlockingChecks, CheckName, Requirement};
///
/// let gate = CheckName::parse("Preview CI Gate").unwrap();
/// let policy = BlockingChecks::from_rule(Requirement::Required, vec![gate]);
///
/// let BlockingChecks::Only { required, .. } = policy else { panic!() };
/// let _smuggled = BlockingChecks::Only { required };
/// ```
///
/// [`Every`]: BlockingChecks::Every
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockingChecks {
    /// Only these checks can stop a merge. Everything else is advisory: it
    /// runs, it can go red, and the merge button ignores it.
    ///
    /// `#[non_exhaustive]` so that no other crate can build one. Reading the
    /// set stays open; forging one does not, which is what stops a required
    /// set being moved onto a rule that never enforced it.
    #[non_exhaustive]
    Only {
        /// The checks this rule names, none of which is optional.
        required: RequiredChecks,
    },
    /// Every failing check is treated as blocking.
    ///
    /// This is the fallback for a policy that told us nothing — no rule covers
    /// the branch, or the rule does not enforce status checks, or it enforces
    /// them without naming any. Silence means "we cannot tell", and a check
    /// wrongly called blocking costs a second look, where a check wrongly
    /// called advisory hides the reason a pull request is stuck.
    Every,
}

impl BlockingChecks {
    /// Reads the status-check half of a branch protection rule.
    ///
    /// Both halves are taken together on purpose, and this is the only way to
    /// build a [`BlockingChecks::Only`]. `requiresStatusChecks` and the context
    /// list are separate fields, so nothing guarantees the list is empty when
    /// the flag is false; a caller that consulted only the list could enforce a
    /// policy the repository has switched off. `enforcement` decides; `required`
    /// is only consulted when it says [`Requirement::Required`], and an empty
    /// list collapses to [`BlockingChecks::Every`] rather than to "nothing
    /// blocks".
    ///
    /// # Examples
    ///
    /// ```
    /// use prtrack_core::{BlockingChecks, CheckName, Requirement};
    ///
    /// let gate = CheckName::parse("Preview CI Gate").unwrap();
    /// let flaky = CheckName::parse("codecov/patch").unwrap();
    ///
    /// let enforced = BlockingChecks::from_rule(Requirement::Required, vec![gate.clone()]);
    /// assert!(enforced.blocks_merge_when_failing(&gate));
    /// assert!(!enforced.blocks_merge_when_failing(&flaky));
    ///
    /// // Switched off, so the names it still lists mean nothing and the
    /// // fallback takes over.
    /// let off = BlockingChecks::from_rule(Requirement::NotRequired, vec![gate]);
    /// assert!(off.blocks_merge_when_failing(&flaky));
    /// ```
    #[must_use]
    pub fn from_rule(enforcement: Requirement, required: Vec<CheckName>) -> Self {
        match enforcement {
            Requirement::Required => RequiredChecks::new(required)
                .map_or(Self::Every, |required| Self::Only { required }),
            Requirement::NotRequired => Self::Every,
        }
    }

    /// Whether a failure of `check` can hold the merge back.
    ///
    /// This asks about the policy, not about the run: it says nothing about
    /// whether `check` has failed, or has even reported. A caller that has a
    /// failing check uses this to decide whether the failure matters.
    #[must_use]
    pub fn blocks_merge_when_failing(&self, check: &CheckName) -> bool {
        match self {
            Self::Only { required } => required.as_slice().contains(check),
            Self::Every => true,
        }
    }
}

/// A repository's branch protection rule, parsed into the questions the board
/// needs to ask of it.
///
/// One value covers both "this branch is protected, here is how" and "no rule
/// covers this branch": the second is [`PolicyRules::unprotected`], not
/// `Option<PolicyRules>`. An `Option` would push the fallback onto every
/// consumer, and a consumer that forgot it would report a stuck pull request
/// as merge-ready.
///
/// The fields are public so that a rule can be built by naming each one, but
/// prefer `..PolicyRules::unprotected()` for the rest: it keeps a construction
/// site honest about what the rule actually demands, and it does not break when
/// GitHub grows another protection setting.
///
/// # Examples
///
/// ```
/// use prtrack_core::{BlockingChecks, CheckName, PolicyRules, Requirement, StaleReviewHandling};
///
/// let rule = PolicyRules {
///     blocking_checks: BlockingChecks::from_rule(
///         Requirement::Required,
///         vec![CheckName::parse("Preview CI Gate").unwrap()],
///     ),
///     required_approvals: 1,
///     conversation_resolution: Requirement::Required,
///     code_owner_reviews: Requirement::Required,
///     up_to_date_branch: Requirement::NotRequired,
///     stale_reviews: StaleReviewHandling::DismissOnPush,
/// };
///
/// let advisory = CheckName::parse("codecov/patch").unwrap();
/// assert!(!rule.blocks_merge_when_failing(&advisory));
/// assert!(PolicyRules::unprotected().blocks_merge_when_failing(&advisory));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyRules {
    /// Which failing checks can hold a merge back.
    pub blocking_checks: BlockingChecks,
    /// How many approving reviews the rule demands.
    ///
    /// Zero covers both halves of GitHub's `requiresApprovingReviews` pair: a
    /// rule that does not ask for approvals at all, and one that asks for none.
    /// They are the same policy, so they are the same value.
    pub required_approvals: u32,
    /// Whether every review thread must be resolved before merging.
    pub conversation_resolution: Requirement,
    /// Whether an approval must come from an owner of the changed paths.
    pub code_owner_reviews: Requirement,
    /// Whether the branch must be up to date with its base before merging.
    ///
    /// GitHub calls this `requiresStrictStatusChecks`. When it is
    /// [`Requirement::NotRequired`], being behind the base branch is not on its
    /// own a reason a pull request cannot merge.
    pub up_to_date_branch: Requirement,
    /// What a push does to approvals the pull request already collected.
    pub stale_reviews: StaleReviewHandling,
}

impl PolicyRules {
    /// The policy for a branch no protection rule covers.
    ///
    /// That is either a repository with no protection at all, or a pull request
    /// aimed at a branch outside every rule's pattern, as a stacked pull request
    /// based on another feature branch is. Matching the base ref against the
    /// pattern is the caller's job; the answer "no rule applies" is this value.
    ///
    /// Nothing is demanded — no approvals, no resolved threads, no code owner —
    /// except the one thing silence does imply: every failing check is treated
    /// as blocking, because a policy we could not read cannot tell us which
    /// failures are harmless.
    ///
    /// This is also the base for building a real rule:
    /// `PolicyRules { required_approvals: 1, ..PolicyRules::unprotected() }`.
    ///
    /// It answers "what is required here", not "was a rule found". A real rule
    /// that demands nothing is *equal* to this value, so `policy ==
    /// PolicyRules::unprotected()` tests that nothing is demanded rather than
    /// that no rule exists — the two are deliberately indistinguishable, since
    /// they lead to identical verdicts. Anything needing the difference must
    /// keep it outside this type.
    ///
    /// # Examples
    ///
    /// ```
    /// use prtrack_core::{CheckName, PolicyRules};
    ///
    /// let anything = CheckName::parse("codecov/patch").unwrap();
    /// assert!(PolicyRules::unprotected().blocks_merge_when_failing(&anything));
    /// assert_eq!(PolicyRules::unprotected().required_approvals, 0);
    /// ```
    #[must_use]
    pub const fn unprotected() -> Self {
        Self {
            blocking_checks: BlockingChecks::Every,
            required_approvals: 0,
            conversation_resolution: Requirement::NotRequired,
            code_owner_reviews: Requirement::NotRequired,
            up_to_date_branch: Requirement::NotRequired,
            stale_reviews: StaleReviewHandling::Keep,
        }
    }

    /// Whether a failure of `check` can hold the merge back under this policy.
    ///
    /// The fallback for a policy that names no required checks is answered
    /// here, once, rather than at each call site. See
    /// [`BlockingChecks::blocks_merge_when_failing`], which this defers to.
    #[must_use]
    pub fn blocks_merge_when_failing(&self, check: &CheckName) -> bool {
        self.blocking_checks.blocks_merge_when_failing(check)
    }
}

#[cfg(test)]
mod tests {
    use super::{BlockingChecks, PolicyRules, RequiredChecks, Requirement, StaleReviewHandling};
    use crate::types::CheckName;

    /// The four contexts `Example-Org/example-repo` requires on `main`, read from
    /// the live rule, against a median of 36 checks per pull request — 6 to 76
    /// across the 37 open when it was measured. Anything outside this list can
    /// fail all it likes without the merge button caring.
    const REQUIRED_ON_MAIN: [&str; 4] = [
        "Preview CI Gate",
        "Validate PR Title",
        "Socket Security: Pull Request Alerts",
        "semgrep-cloud-platform/scan",
    ];

    /// Names shaped like the checks that run beside the required ones but are
    /// listed in no rule.
    const ADVISORY_ON_MAIN: [&str; 3] = [
        "build / test (ubuntu-latest)",
        "ci/circleci: build",
        "codecov/patch",
    ];

    /// Parses fixture names into check names, failing loudly rather than
    /// silently dropping one: a required list built from a name that did not
    /// parse would leave the assertions below asserting nothing.
    fn contexts(raw: &[&str]) -> Vec<CheckName> {
        let parsed: Vec<CheckName> = raw
            .iter()
            .filter_map(|name| CheckName::parse(name).ok())
            .collect();
        assert_eq!(
            parsed.len(),
            raw.len(),
            "every fixture name must parse or this test proves nothing: {raw:?}"
        );
        parsed
    }

    // Paired with `a_check_the_rule_does_not_name_is_advisory_however_red_it_goes`,
    // and worth little alone. This one asserts `true` for every input, so an
    // implementation that blocked on everything unconditionally would satisfy
    // it — its non-vacuity is borrowed entirely from the advisory test, which
    // is the only thing proving the fallback is a decision rather than a
    // constant. Weaken that one and this becomes the most important-looking
    // vacuous test in the crate.
    #[test]
    fn silence_in_a_policy_means_every_failing_check_blocks() {
        let cases: [(&str, PolicyRules); 3] = [
            (
                "no branch protection rule at all",
                PolicyRules::unprotected(),
            ),
            (
                "a rule that does not enforce status checks, whatever names it still lists",
                PolicyRules {
                    blocking_checks: BlockingChecks::from_rule(
                        Requirement::NotRequired,
                        contexts(&REQUIRED_ON_MAIN),
                    ),
                    ..PolicyRules::unprotected()
                },
            ),
            (
                "a rule that enforces status checks but names none",
                PolicyRules {
                    blocking_checks: BlockingChecks::from_rule(Requirement::Required, Vec::new()),
                    ..PolicyRules::unprotected()
                },
            ),
        ];

        for (policy_description, policy) in cases {
            for raw in REQUIRED_ON_MAIN.iter().chain(ADVISORY_ON_MAIN.iter()) {
                assert_eq!(
                    CheckName::parse(raw).map(|name| policy.blocks_merge_when_failing(&name)),
                    Ok(true),
                    "with {policy_description}, {raw:?} must be treated as blocking"
                );
            }
        }
    }

    #[test]
    fn an_empty_required_list_is_not_a_set_of_required_checks() {
        assert_eq!(RequiredChecks::new(Vec::new()), None);
    }

    #[test]
    fn a_context_the_rule_lists_twice_is_held_once() {
        let listed_twice = contexts(&["Preview CI Gate", "Validate PR Title", "Preview CI Gate"]);
        let once_each = contexts(&["Preview CI Gate", "Validate PR Title"]);
        assert_eq!(
            RequiredChecks::new(listed_twice)
                .as_ref()
                .map(RequiredChecks::as_slice),
            Some(once_each.as_slice()),
            "a repeat must not inflate a count taken from the required list"
        );
    }

    #[test]
    fn a_branch_no_rule_covers_demands_nothing_except_its_checks() {
        // The check fallback is strict because a policy we could not read
        // cannot tell us which failures are harmless. Every other field goes
        // the other way: inventing an approval or a resolved-thread
        // requirement out of an absent rule would report a mergeable pull
        // request as blocked, and a false alarm is the expensive mistake here.
        let unprotected = PolicyRules::unprotected();
        assert_eq!(unprotected.required_approvals, 0);
        assert_eq!(
            unprotected.conversation_resolution,
            Requirement::NotRequired
        );
        assert_eq!(unprotected.code_owner_reviews, Requirement::NotRequired);
        assert_eq!(unprotected.up_to_date_branch, Requirement::NotRequired);
        assert_eq!(unprotected.stale_reviews, StaleReviewHandling::Keep);
    }

    /// The rule `Example-Org/example-repo` enforces on `main`, which is the one
    /// case with a required list to get wrong.
    fn example_org_main() -> PolicyRules {
        PolicyRules {
            blocking_checks: BlockingChecks::from_rule(
                Requirement::Required,
                contexts(&REQUIRED_ON_MAIN),
            ),
            required_approvals: 1,
            conversation_resolution: Requirement::Required,
            code_owner_reviews: Requirement::Required,
            up_to_date_branch: Requirement::NotRequired,
            stale_reviews: StaleReviewHandling::DismissOnPush,
        }
    }

    #[test]
    fn a_check_the_rule_names_blocks_when_it_fails() {
        let policy = example_org_main();
        for raw in REQUIRED_ON_MAIN {
            assert_eq!(
                CheckName::parse(raw).map(|name| policy.blocks_merge_when_failing(&name)),
                Ok(true),
                "{raw:?} is on the required list, so failing it must block"
            );
        }
    }

    // The other half of `silence_in_a_policy_means_every_failing_check_blocks`.
    // That test cannot fail against an implementation treating everything as
    // blocking; this one is what rules that out, so it must keep asserting
    // `false` for a check no rule names. Do not weaken it without giving its
    // partner a way to fail on its own.
    #[test]
    fn a_check_the_rule_does_not_name_is_advisory_however_red_it_goes() {
        let policy = example_org_main();
        for raw in ADVISORY_ON_MAIN {
            assert_eq!(
                CheckName::parse(raw).map(|name| policy.blocks_merge_when_failing(&name)),
                Ok(false),
                "{raw:?} is on no required list, so the merge button ignores it"
            );
        }
    }

    #[test]
    fn matching_a_required_check_is_case_sensitive_because_github_compares_it_literally() {
        let policy = example_org_main();
        for raw in [
            "preview ci gate",
            "PREVIEW CI GATE",
            "Preview Ci Gate",
            "Semgrep-Cloud-Platform/Scan",
        ] {
            assert_eq!(
                CheckName::parse(raw).map(|name| policy.blocks_merge_when_failing(&name)),
                Ok(false),
                "{raw:?} differs in case from anything the rule names, so it is a different check"
            );
        }
    }

    #[test]
    fn padding_around_a_name_does_not_change_the_verdict_on_either_side() {
        // `CheckName` trims, so both sides of the comparison normalise the same
        // way and a name that arrived quoted or line-wrapped still matches. It
        // is only the padding that is forgiven: an advisory check does not
        // become blocking by acquiring a space.
        let policy = example_org_main();
        for (raw, blocks) in [
            ("  Preview CI Gate", true),
            ("Preview CI Gate\n", true),
            (" \tsemgrep-cloud-platform/scan \r\n", true),
            ("  codecov/patch  ", false),
            (" preview ci gate ", false),
        ] {
            assert_eq!(
                CheckName::parse(raw).map(|name| policy.blocks_merge_when_failing(&name)),
                Ok(blocks),
                "padding must not change the verdict on {raw:?}"
            );
        }

        // And the same holds when it is the rule's own list that arrived padded.
        let padded = PolicyRules {
            blocking_checks: BlockingChecks::from_rule(
                Requirement::Required,
                contexts(&["  Preview CI Gate\n"]),
            ),
            ..PolicyRules::unprotected()
        };
        assert_eq!(
            CheckName::parse("Preview CI Gate").map(|name| padded.blocks_merge_when_failing(&name)),
            Ok(true),
            "a padded required context must still match the name a check reports"
        );
    }
}
