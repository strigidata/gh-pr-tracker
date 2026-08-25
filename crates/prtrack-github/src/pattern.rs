//! Matching a branch name against a protection rule's pattern.
//!
//! # Why this exists here
//!
//! `PolicyRules` deliberately has no pattern field: it answers "what is
//! required here", and *which* rule applies to a given pull request is a
//! question about GitHub's configuration, not about the domain. So the core
//! left base-ref matching to its caller, and this crate is the caller.
//!
//! Getting it wrong is silent and wrong in the dangerous direction. A stacked
//! pull request aimed at another feature branch is governed by no rule, and
//! must fall back to "every failing check blocks". If it were matched against
//! `main`'s rule instead, its failing checks would be sorted into blocking and
//! advisory by a list that does not apply to it, and the advisory ones would be
//! rendered as harmless. Nothing downstream can detect that.
//!
//! # The supported syntax, and what happens outside it
//!
//! GitHub's branch protection patterns use `fnmatch` syntax. This implements
//! the part that repositories actually use:
//!
//! | Pattern | Matches |
//! | --- | --- |
//! | `main` | exactly that branch |
//! | `release/*` | one segment after `release/`, not `release/1/2` |
//! | `release/**` | any number of segments, including none |
//! | `v?` | `v1`, not `v10` and not `v/` |
//!
//! Anything using a character class — `[0-9]`, `[!a-z]` — is **not** matched,
//! and a rule carrying one therefore governs nothing. That is the safe
//! direction: a pull request judged against no rule has every failing check
//! treated as blocking, which over-reports what is holding it up. The opposite
//! mistake hides a blocker.

/// The most wildcards a pattern may carry before it is refused.
///
/// Matching backtracks, so a pattern of alternating wildcards against a long
/// branch name is exponential. Patterns come from a repository's own settings
/// rather than from a stranger, but this crate runs in a poll loop and a
/// repository nobody vetted is exactly what it gets pointed at. No real branch
/// pattern comes close to this.
const MAX_WILDCARDS: usize = 8;

/// Whether `branch` is governed by a rule protecting `pattern`.
///
/// Returns `false` for a pattern this crate cannot read, which leaves the pull
/// request ungoverned. See the module documentation for why that is the safe
/// answer.
pub(crate) fn matches(pattern: &str, branch: &str) -> bool {
    if pattern.contains(['[', ']']) || pattern.matches('*').count() > MAX_WILDCARDS {
        return false;
    }
    let pattern: Vec<char> = pattern.chars().collect();
    let branch: Vec<char> = branch.chars().collect();
    matches_chars(&pattern, &branch)
}

/// The recursive half of [`matches`], over characters so that a multi-byte
/// branch name cannot be split through the middle of one.
fn matches_chars(pattern: &[char], branch: &[char]) -> bool {
    let Some((head, rest)) = pattern.split_first() else {
        return branch.is_empty();
    };

    match *head {
        '*' => {
            if let Some(('*', after)) = rest.split_first() {
                // `**` crosses separators, so every suffix is a candidate.
                (0..=branch.len())
                    .any(|skip| matches_chars(after, branch.get(skip..).unwrap_or_default()))
            } else {
                // `*` stops at the first separator, so only the suffixes within
                // this segment are.
                let segment = branch
                    .iter()
                    .position(|c| *c == '/')
                    .unwrap_or(branch.len());
                (0..=segment)
                    .any(|skip| matches_chars(rest, branch.get(skip..).unwrap_or_default()))
            }
        }
        '?' => {
            matches!(branch.split_first(), Some((c, tail)) if *c != '/' && matches_chars(rest, tail))
        }
        expected => {
            matches!(branch.split_first(), Some((c, tail)) if *c == expected && matches_chars(rest, tail))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn a_literal_pattern_matches_only_itself() {
        assert!(matches("main", "main"));
        for branch in ["mainline", "ain", "Main", "main/x", "x/main", ""] {
            assert!(
                !matches("main", branch),
                "expected `main` not to govern {branch:?}"
            );
        }
    }

    #[test]
    fn a_star_matches_within_one_segment_only() {
        // `release/*` governing `release/1/hotfix` would be a rule reaching a
        // branch its author did not point it at.
        for branch in ["release/1", "release/candidate", "release/"] {
            assert!(
                matches("release/*", branch),
                "expected to govern {branch:?}"
            );
        }
        for branch in ["release", "release/1/2", "other/1"] {
            assert!(
                !matches("release/*", branch),
                "expected not to govern {branch:?}"
            );
        }
    }

    #[test]
    fn a_double_star_crosses_segments() {
        for branch in ["release", "release/1", "release/1/2/3"] {
            assert!(
                matches("release**", branch),
                "expected to govern {branch:?}"
            );
        }
        for branch in ["main", "prerelease"] {
            assert!(
                !matches("release**", branch),
                "expected not to govern {branch:?}"
            );
        }
    }

    #[test]
    fn a_bare_double_star_governs_everything() {
        for branch in ["main", "a/b/c/d", ""] {
            assert!(matches("**", branch), "expected `**` to govern {branch:?}");
        }
    }

    #[test]
    fn a_question_mark_matches_one_character_that_is_not_a_separator() {
        assert!(matches("v?", "v1"));
        for branch in ["v", "v10", "v/"] {
            assert!(!matches("v?", branch), "expected not to govern {branch:?}");
        }
    }

    #[test]
    fn a_pattern_this_crate_cannot_read_governs_nothing() {
        // Refusing leaves the pull request ungoverned, so every failing check
        // blocks. Over-reporting what is holding a pull request up costs a
        // second look; under-reporting hides the reason it is stuck.
        for pattern in ["release/[0-9]", "v[!0-9]", "]"] {
            assert!(
                !matches(pattern, "release/1"),
                "expected {pattern:?} to be refused rather than guessed at"
            );
        }
    }

    #[test]
    fn a_pattern_dense_with_wildcards_is_refused_rather_than_explored() {
        // Nine wildcards against a long name is exponential backtracking in a
        // poll loop. No real branch pattern looks like this.
        let dense = "*a*a*a*a*a*a*a*a*b";
        let long = "a".repeat(64);
        assert!(!matches(dense, &long));
    }

    #[test]
    fn matching_is_case_sensitive_because_git_refs_are() {
        // `Main` and `main` are two different refs, and a rule protecting one
        // says nothing about the other.
        assert!(!matches("Main", "main"));
        assert!(!matches("main", "MAIN"));
    }
}
