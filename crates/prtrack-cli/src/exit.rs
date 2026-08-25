//! What the process tells whoever ran it.
//!
//! An exit code is the only part of this program a script can read reliably,
//! and the distinction it has to carry is the one the whole project turns on:
//! a board with nothing on it and a board that could not be fetched print
//! almost the same thing, and only the code says which happened.

use prtrack_github::GitHubError;

/// The process's answer, as a named outcome rather than a number at a call
/// site.
///
/// The numbers are a contract. A wrapper script keys off them, so renumbering
/// is a breaking change to code nobody here can see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// The command printed what was asked of it and did no work.
    ///
    /// `--help` and `--version`. Shares `0` with [`Exit::Board`] because both
    /// succeeded, but kept separate so the type does not have to claim a board
    /// was fetched when none was — the alternative is returning [`Exit::Board`]
    /// for a run that never spoke to GitHub.
    Informational,
    /// The board was fetched and has at least one pull request on it.
    Board,
    /// The fetch succeeded and the repository has nothing open.
    ///
    /// Not a failure. Separate from [`Exit::Board`] so that a script polling
    /// for work can tell "nothing to do" from "something to do" without
    /// parsing stdout, and from a failure without guessing.
    NothingOpen,
    /// The command line was wrong, or named a repository GitHub will not show
    /// us. Nothing about retrying helps.
    Usage,
    /// GitHub refused the credentials. Retrying with the same token is
    /// pointless, and hammering it is how a token gets rate limited too.
    Auth,
    /// GitHub or the network failed. This is the one worth retrying.
    Api,
}

impl Exit {
    /// The number the process exits with.
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Informational | Self::Board => 0,
            Self::NothingOpen => 1,
            Self::Usage => 2,
            Self::Auth => 3,
            Self::Api => 4,
        }
    }

    /// Which kind of failure a transport error is.
    ///
    /// The split is by what the reader should *do*, not by where the failure
    /// arose: credentials they must change, a command line they must change,
    /// or a call that might work next time.
    #[must_use]
    pub const fn of(error: &GitHubError) -> Self {
        match error {
            GitHubError::MissingToken
            | GitHubError::UnusableToken
            | GitHubError::Unauthorized
            | GitHubError::Forbidden => Self::Auth,

            // GitHub answers identically for a repository that does not exist
            // and one this token cannot see — deliberately, so a private name
            // cannot be probed. Neither is fixed by retrying and both are
            // fixed by the person at the keyboard, so this is the bucket that
            // says "change something". The message has to name both causes,
            // because the code cannot.
            GitHubError::RepositoryUnavailable { .. } | GitHubError::UnusableEndpoint => {
                Self::Usage
            }

            // Everything else. The known variants are named so a reader can
            // see what lands here, and the trailing wildcard is in the *same*
            // arm because `GitHubError` is `#[non_exhaustive]`: the language
            // requires it, so no arrangement of arms gets a compile error when
            // upstream adds one. Naming them is documentation, not a check.
            //
            // An API failure is the safe reading for an unrecognised one: a
            // wrapper retries and wastes a request, where calling it a usage
            // error would send somebody to fix a command line that is fine.
            GitHubError::Transport(_)
            | GitHubError::RateLimited
            | GitHubError::UnexpectedStatus { .. }
            | GitHubError::Decode(_)
            | GitHubError::Malformed(_)
            | GitHubError::GraphQl { .. }
            | GitHubError::GaveUp { .. }
            | GitHubError::TooManyPages { .. }
            | _ => Self::Api,
        }
    }
}

impl From<Exit> for std::process::ExitCode {
    fn from(exit: Exit) -> Self {
        Self::from(exit.code())
    }
}

#[cfg(test)]
mod tests {
    use super::Exit;
    use prtrack_core::RepoRef;
    use prtrack_github::GitHubError;

    #[test]
    fn the_codes_are_the_contract_and_do_not_move() {
        // A script keys off these numbers. Renumbering them is a breaking
        // change to every wrapper anybody has written, so they are pinned
        // here rather than left to the order of the variants.
        for (exit, code) in [
            // `--help` and `--version` succeeded. A shell that reads 2 as "you
            // typed it wrong" would otherwise be told so by the one command
            // that cannot be typed wrong.
            (Exit::Informational, 0),
            (Exit::Board, 0),
            (Exit::NothingOpen, 1),
            (Exit::Usage, 2),
            (Exit::Auth, 3),
            (Exit::Api, 4),
        ] {
            assert_eq!(exit.code(), code, "{exit:?}");
        }
    }

    #[test]
    fn success_and_an_empty_board_are_told_apart() {
        // The distinction the whole taxonomy exists for. A board with nothing
        // on it and a board that could not be fetched look identical on
        // stdout, and the exit code is where a script learns which it got.
        assert_ne!(Exit::Board.code(), Exit::NothingOpen.code());
    }

    #[test]
    fn a_refused_credential_is_told_apart_from_a_failed_call() {
        // Retrying helps one and not the other. A wrapper that treats every
        // failure as transient hammers GitHub with a token it has already been
        // told is no good.
        for refused in [
            GitHubError::MissingToken,
            GitHubError::UnusableToken,
            GitHubError::Unauthorized,
            GitHubError::Forbidden,
        ] {
            assert_eq!(Exit::of(&refused), Exit::Auth, "{refused:?}");
        }
        for failed in [
            GitHubError::RateLimited,
            GitHubError::TooManyPages { pages: 11 },
        ] {
            assert_eq!(Exit::of(&failed), Exit::Api, "{failed:?}");
        }
    }

    #[test]
    fn a_repository_github_will_not_show_us_is_reported_as_a_usage_error() {
        // GitHub answers the same way for a repository that does not exist and
        // one the token cannot see, deliberately, so that a private name
        // cannot be probed. Neither is fixed by retrying and both are fixed by
        // the person at the keyboard, so it goes in the bucket that says "you
        // need to change something" -- and the message names both causes,
        // because the exit code cannot.
        let classified = RepoRef::parse("octo-org/example")
            .map(|repo| Exit::of(&GitHubError::RepositoryUnavailable { repo }));
        assert_eq!(classified, Ok(Exit::Usage));
    }

    #[test]
    fn a_failure_that_might_clear_is_told_apart_from_one_that_will_not() {
        // The rest of the taxonomy, so that every variant this build knows
        // about is classified by a test rather than by reading the match.
        for transient in [
            GitHubError::UnexpectedStatus { status: 503 },
            GitHubError::TooManyPages { pages: 11 },
            // The one a poll loop actually meets: a 504 that did not clear.
            GitHubError::GaveUp {
                attempts: 3,
                source: Box::new(GitHubError::UnexpectedStatus { status: 504 }),
            },
        ] {
            assert_eq!(Exit::of(&transient), Exit::Api, "{transient:?}");
        }
        assert_eq!(Exit::of(&GitHubError::UnusableEndpoint), Exit::Usage);
    }

    // The wildcard arm in `Exit::of` is **not** tested, and cannot be from
    // here: `GitHubError` is `#[non_exhaustive]`, so the arm exists for
    // variants that do not exist yet, and every variant that does exist is
    // named above it. Nothing can construct the case it catches until upstream
    // adds one — at which point this file should name it explicitly and the
    // wildcard should go back to being unreachable.
}
