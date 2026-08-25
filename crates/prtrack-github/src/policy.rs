//! Which branch protection rule judges a pull request, and what it demands.
//!
//! # The half the core deliberately left out
//!
//! `PolicyRules` answers "what is required here" and carries no pattern,
//! because *which* rule applies is a question about GitHub's configuration
//! rather than about the domain. Resolving it is this module's job, and it is
//! the one place where a stacked pull request stops being judged by `main`'s
//! rule. See [`crate::pattern`] for why getting that wrong is silent.

use prtrack_core::{BlockingChecks, CheckName, PolicyRules, Requirement, StaleReviewHandling};

use crate::error::ConversionError;
use crate::pattern;
use crate::wire;

/// The rules a pull request based on `base_ref` is judged against.
///
/// A branch no rule covers yields [`PolicyRules::unprotected`], which is not
/// "nothing is required" so much as "the policy told us nothing" — and under it
/// every failing check blocks, because a policy we could not read cannot say
/// which failures are harmless.
///
/// # Which rule, when more than one matches
///
/// A rule naming the branch outright wins over a pattern that merely covers it,
/// so `main` beats `**`. Beyond that the first match in the order GitHub
/// returned them is taken.
///
/// That tie-break is a simplification and worth knowing about: GitHub's own
/// resolution for two overlapping wildcard rules is not documented precisely
/// enough to reproduce confidently, and guessing would be worse than saying so.
/// A repository with two wildcard rules over one branch may therefore be judged
/// by the other one. Exact-over-wildcard covers the shape repositories actually
/// have, including every repository this has been run against.
///
/// # Errors
///
/// Returns [`ConversionError::ProtectionRule`] for a required context that is
/// not a check name, and [`ConversionError::ApprovalCount`] for an approval
/// count outside what a count can be.
pub(crate) fn rules_for(
    base_ref: &str,
    rules: &[wire::BranchProtectionRule],
) -> Result<PolicyRules, ConversionError> {
    let named_outright = rules.iter().find(|rule| rule.pattern == base_ref);
    let covered_by_pattern = || {
        rules
            .iter()
            .find(|rule| pattern::matches(&rule.pattern, base_ref))
    };

    named_outright
        .or_else(covered_by_pattern)
        .map_or_else(|| Ok(PolicyRules::unprotected()), read_rule)
}

/// The domain's word for a flag the API reports as a boolean.
const fn requirement_of(flag: wire::Flag) -> Requirement {
    match flag {
        wire::Flag::Yes => Requirement::Required,
        wire::Flag::No => Requirement::NotRequired,
    }
}

/// Reads one rule into the questions the board asks of it.
fn read_rule(rule: &wire::BranchProtectionRule) -> Result<PolicyRules, ConversionError> {
    let contexts = rule
        .required_status_check_contexts
        .iter()
        .flatten()
        .flatten()
        .map(|context| {
            CheckName::parse(context).map_err(|source| ConversionError::ProtectionRule {
                pattern: rule.pattern.clone(),
                source,
            })
        })
        .collect::<Result<Vec<CheckName>, ConversionError>>()?;

    let demanded = match rule.requires_approving_reviews {
        wire::Flag::Yes => rule.required_approving_review_count.unwrap_or(0),
        wire::Flag::No => 0,
    };
    let required_approvals =
        u32::try_from(demanded).map_err(|_source| ConversionError::ApprovalCount {
            pattern: rule.pattern.clone(),
            count: demanded,
        })?;

    Ok(PolicyRules {
        blocking_checks: BlockingChecks::from_rule(
            requirement_of(rule.requires_status_checks),
            contexts,
        ),
        required_approvals,
        conversation_resolution: requirement_of(rule.requires_conversation_resolution),
        code_owner_reviews: requirement_of(rule.requires_code_owner_reviews),
        up_to_date_branch: requirement_of(rule.requires_strict_status_checks),
        stale_reviews: match rule.dismisses_stale_reviews {
            wire::Flag::Yes => StaleReviewHandling::DismissOnPush,
            wire::Flag::No => StaleReviewHandling::Keep,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::rules_for;
    use crate::error::ConversionError;
    use crate::wire::{BranchProtectionRule, Flag};
    use prtrack_core::{BlockingChecks, PolicyRules, Requirement, StaleReviewHandling};

    /// A rule protecting `pattern` that demands everything it can.
    fn strict(pattern: &str, contexts: &[&str]) -> BranchProtectionRule {
        BranchProtectionRule {
            pattern: pattern.to_owned(),
            requires_approving_reviews: Flag::Yes,
            required_approving_review_count: Some(1),
            requires_conversation_resolution: Flag::Yes,
            requires_status_checks: Flag::Yes,
            requires_strict_status_checks: Flag::Yes,
            required_status_check_contexts: Some(
                contexts.iter().map(|c| Some((*c).to_owned())).collect(),
            ),
            requires_code_owner_reviews: Flag::Yes,
            dismisses_stale_reviews: Flag::Yes,
        }
    }

    /// The names a resolved policy treats as able to block, or `None` where it
    /// falls back to treating every failure as blocking.
    fn blocking(policy: &PolicyRules) -> Option<Vec<String>> {
        match &policy.blocking_checks {
            BlockingChecks::Only { required, .. } => Some(
                required
                    .as_slice()
                    .iter()
                    .map(|name| name.as_str().to_owned())
                    .collect(),
            ),
            BlockingChecks::Every => None,
        }
    }

    #[test]
    fn a_branch_the_rule_protects_is_judged_by_it() {
        let rules = [strict("main", &["Preview CI Gate"])];
        let policy = rules_for("main", &rules);

        assert_eq!(
            policy.as_ref().ok().and_then(blocking),
            Some(vec!["Preview CI Gate".to_owned()]),
            "the rule names one required check, so only that one blocks"
        );
        assert_eq!(policy.as_ref().map(|p| p.required_approvals).ok(), Some(1));
        assert_eq!(
            policy.as_ref().map(|p| p.conversation_resolution).ok(),
            Some(Requirement::Required)
        );
        assert_eq!(
            policy.as_ref().map(|p| p.up_to_date_branch).ok(),
            Some(Requirement::Required)
        );
        assert_eq!(
            policy.as_ref().map(|p| p.stale_reviews).ok(),
            Some(StaleReviewHandling::DismissOnPush)
        );
    }

    #[test]
    fn a_stacked_pull_request_is_judged_by_no_rule_at_all() {
        // The case the whole module exists for. A pull request based on another
        // feature branch is outside every pattern, and judging it against
        // `main`'s rule would sort its failing checks into blocking and
        // advisory by a list it is not subject to — rendering the advisory ones
        // as harmless when nothing says they are.
        let rules = [strict("main", &["Preview CI Gate"])];
        let policy = rules_for("feat/stacked-on-another-branch", &rules);

        assert_eq!(
            policy.ok(),
            Some(PolicyRules::unprotected()),
            "no rule covers it, so nothing is advisory and nothing is demanded"
        );
    }

    #[test]
    fn a_wildcard_rule_governs_the_branches_it_covers() {
        let rules = [strict("release/*", &["Preview CI Gate"])];

        assert_eq!(
            rules_for("release/1.2", &rules)
                .as_ref()
                .ok()
                .and_then(blocking),
            Some(vec!["Preview CI Gate".to_owned()])
        );
        assert_eq!(
            rules_for("main", &rules).ok(),
            Some(PolicyRules::unprotected()),
            "a branch outside the pattern is not covered by it"
        );
    }

    #[test]
    fn an_exact_rule_wins_over_a_wildcard_that_also_matches() {
        // Both cover `main`. Taking whichever GitHub happened to list first
        // would make the board depend on the order of an API response.
        let rules = [
            strict("**", &["Everything Gate"]),
            strict("main", &["Preview CI Gate"]),
        ];
        assert_eq!(
            rules_for("main", &rules).as_ref().ok().and_then(blocking),
            Some(vec!["Preview CI Gate".to_owned()]),
            "the rule naming the branch outright is the one that applies"
        );
        assert_eq!(
            rules_for("other", &rules).as_ref().ok().and_then(blocking),
            Some(vec!["Everything Gate".to_owned()]),
            "and the wildcard still covers what it alone matches"
        );
    }

    #[test]
    fn a_rule_that_does_not_enforce_checks_makes_every_failure_blocking() {
        // `requiresStatusChecks` and the context list are separate fields, so a
        // switched-off rule can still be carrying names. Reading the list alone
        // would enforce a policy the repository has turned off — and, worse,
        // call every check outside that stale list advisory.
        let mut rule = strict("main", &["Preview CI Gate"]);
        rule.requires_status_checks = Flag::No;

        assert_eq!(
            rules_for("main", &[rule]).as_ref().ok().and_then(blocking),
            None,
            "silence about checks means we cannot tell, so everything blocks"
        );
    }

    #[test]
    fn a_rule_naming_no_checks_makes_every_failure_blocking() {
        for contexts in [Some(vec![]), None] {
            let mut rule = strict("main", &[]);
            rule.required_status_check_contexts = contexts.clone();

            assert_eq!(
                rules_for("main", &[rule]).as_ref().ok().and_then(blocking),
                None,
                "an empty required list is not \"nothing blocks\": {contexts:?}"
            );
        }
    }

    #[test]
    fn a_rule_that_asks_for_no_approvals_demands_none() {
        // GitHub reports "does not ask for approvals" and "asks for zero" as
        // two different field combinations meaning the same policy.
        let mut off = strict("main", &["Preview CI Gate"]);
        off.requires_approving_reviews = Flag::No;
        off.required_approving_review_count = Some(2);

        assert_eq!(
            rules_for("main", &[off])
                .as_ref()
                .map(|p| p.required_approvals)
                .ok(),
            Some(0),
            "a count left behind by a switched-off requirement demands nothing"
        );
    }

    #[test]
    fn a_rule_that_relaxes_a_requirement_is_read_as_relaxed() {
        let mut relaxed = strict("main", &["Preview CI Gate"]);
        relaxed.requires_conversation_resolution = Flag::No;
        relaxed.requires_code_owner_reviews = Flag::No;
        relaxed.requires_strict_status_checks = Flag::No;
        relaxed.dismisses_stale_reviews = Flag::No;

        let policy = rules_for("main", &[relaxed]);
        assert_eq!(
            policy.as_ref().map(|p| p.conversation_resolution).ok(),
            Some(Requirement::NotRequired)
        );
        assert_eq!(
            policy.as_ref().map(|p| p.code_owner_reviews).ok(),
            Some(Requirement::NotRequired)
        );
        assert_eq!(
            policy.as_ref().map(|p| p.up_to_date_branch).ok(),
            Some(Requirement::NotRequired)
        );
        assert_eq!(
            policy.as_ref().map(|p| p.stale_reviews).ok(),
            Some(StaleReviewHandling::Keep)
        );
    }

    #[test]
    fn a_required_context_that_is_not_a_check_name_is_refused() {
        let rule = strict("main", &["   "]);
        assert!(
            matches!(
                rules_for("main", &[rule]),
                Err(ConversionError::ProtectionRule { .. })
            ),
            "a blank required context could never match a reported check, so \
             accepting it would quietly classify a real failure as advisory"
        );
    }

    #[test]
    fn an_impossible_approval_count_is_refused() {
        let mut rule = strict("main", &["Preview CI Gate"]);
        rule.required_approving_review_count = Some(-1);

        assert!(matches!(
            rules_for("main", &[rule]),
            Err(ConversionError::ApprovalCount { count: -1, .. })
        ));
    }
}
