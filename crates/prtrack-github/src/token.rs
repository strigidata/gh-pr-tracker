//! The credential, and the only shape it is allowed to exist in.

use reqwest::header::HeaderValue;

use crate::error::GitHubError;

/// The environment variable the `gh` CLI reads first.
pub const GH_TOKEN: &str = "GH_TOKEN";

/// The environment variable the `gh` CLI falls back to, and the one GitHub
/// Actions sets.
pub const GITHUB_TOKEN: &str = "GITHUB_TOKEN";

/// The authentication scheme GitHub's GraphQL API expects.
///
/// `bearer` and `token` are both accepted; `bearer` is what GitHub's own
/// documentation uses for the GraphQL endpoint.
const CREDENTIAL_SCHEME: &str = "bearer";

/// The smallest and largest byte a token may contain.
///
/// Visible ASCII only: no space, no control character, nothing above `~`. A
/// real GitHub token is `[A-Za-z0-9_]`, so this rejects nothing legitimate
/// while ruling out the header-splitting and pasted-prefix mistakes.
const FIRST_VISIBLE_ASCII: u8 = 0x21;
/// See [`FIRST_VISIBLE_ASCII`].
const LAST_VISIBLE_ASCII: u8 = 0x7e;

/// A GitHub API token, held only in the form it is sent in.
///
/// # The token never exists as plain text in this crate
///
/// A `Token` stores the finished `Authorization` header value and nothing
/// else, so there is no accessor that could hand the credential back and no
/// second copy to forget about. The value is marked
/// [sensitive](HeaderValue::set_sensitive), which is what makes `reqwest` and
/// `http` print it as a placeholder wherever they debug their headers, and
/// `Debug` here is written by hand for the same reason: a derived one would
/// put the credential into every error chain that happened to include a
/// client.
///
/// # It is never passed on a command line
///
/// The only sources are [`Token::parse`] and [`Token::from_env`]. Nothing in
/// this crate reads a token from an argument, because `argv` is world-readable
/// through `ps`.
pub struct Token {
    credential: HeaderValue,
}

impl std::fmt::Debug for Token {
    /// Prints the type and nothing about the value.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Token").finish_non_exhaustive()
    }
}

impl Token {
    /// Reads a token from raw text, trimming whitespace around it.
    ///
    /// Trimming is deliberate: a token read from a file or pasted from a
    /// terminal routinely arrives with a trailing newline, and refusing it
    /// would surface later as "GitHub rejected the token", which sends the
    /// reader looking in the wrong place.
    ///
    /// # Errors
    ///
    /// Returns [`GitHubError::MissingToken`] when nothing but whitespace was
    /// supplied, and [`GitHubError::UnusableToken`] when what is left contains
    /// anything outside visible ASCII — a space, a newline, a control
    /// character, or any non-ASCII byte. Neither message contains the value.
    ///
    /// # Examples
    ///
    /// ```
    /// use prtrack_github::Token;
    ///
    /// assert!(Token::parse("ghp_examplevalue\n").is_ok());
    /// assert!(Token::parse("   ").is_err());
    /// assert!(Token::parse("bearer ghp_examplevalue").is_err());
    /// ```
    pub fn parse(raw: &str) -> Result<Self, GitHubError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(GitHubError::MissingToken);
        }
        if !trimmed
            .bytes()
            .all(|byte| (FIRST_VISIBLE_ASCII..=LAST_VISIBLE_ASCII).contains(&byte))
        {
            return Err(GitHubError::UnusableToken);
        }

        // Unreachable given the check above, which is the point: the fallible
        // step happens here, once, so that every later use of the credential
        // is infallible and needs no `unwrap`.
        let mut credential = HeaderValue::from_str(&format!("{CREDENTIAL_SCHEME} {trimmed}"))
            .map_err(|_source| GitHubError::UnusableToken)?;
        credential.set_sensitive(true);

        Ok(Self { credential })
    }

    /// Reads a token from the environment, preferring [`GH_TOKEN`] over
    /// [`GITHUB_TOKEN`].
    ///
    /// That is the `gh` CLI's documented precedence. A developer who set both
    /// and is authenticated in `gh` gets the same account here.
    ///
    /// A variable that is set but blank is treated as unset, so exporting an
    /// empty `GH_TOKEN` does not shadow a real `GITHUB_TOKEN`.
    ///
    /// # Errors
    ///
    /// Returns [`GitHubError::MissingToken`] when neither variable names a
    /// non-blank value, and [`GitHubError::UnusableToken`] when the first one
    /// that does cannot be sent as a header. The second case is deliberately
    /// not reported as missing: "set a variable" and "your token is wrong" are
    /// different instructions to the reader.
    pub fn from_env() -> Result<Self, GitHubError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// [`Token::from_env`] with the environment supplied by the caller.
    ///
    /// Split out so the precedence rule can be tested as a pure function.
    /// Mutating the real environment from a test races every other test in the
    /// binary, and a racy test that passes proves nothing.
    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, GitHubError> {
        [GH_TOKEN, GITHUB_TOKEN]
            .into_iter()
            .filter_map(lookup)
            .find(|value| !value.trim().is_empty())
            .ok_or(GitHubError::MissingToken)
            .and_then(|value| Self::parse(&value))
    }

    /// The `Authorization` header value this token is sent as, already marked
    /// sensitive.
    pub(crate) fn credential(&self) -> HeaderValue {
        self.credential.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::{Token, GH_TOKEN, GITHUB_TOKEN};
    use crate::error::GitHubError;
    use reqwest::header::HeaderValue;

    /// A token-shaped string that no real account issued. It is checked for by
    /// substring in the redaction tests, so it must not appear in any message
    /// this crate can produce for an unrelated reason.
    const SECRET: &str = "ghp_0123456789abcdefSECRETzzzz";

    /// The token a lookup returns, or `None` for a variable that is not set.
    fn lookup<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn debug_does_not_print_the_token() {
        let token = Token::parse(SECRET).ok();
        assert!(
            token.is_some(),
            "the token must parse or this proves nothing"
        );
        let rendered = format!("{token:?}");
        assert!(
            !rendered.contains(SECRET),
            "a derived Debug puts the credential into every error chain: {rendered}"
        );
    }

    #[test]
    fn a_blank_token_is_refused() {
        for raw in ["", " ", "\t", "\n", "   \r\n "] {
            assert!(
                matches!(Token::parse(raw), Err(GitHubError::MissingToken)),
                "expected {raw:?} to be refused as blank"
            );
        }
    }

    #[test]
    fn a_token_an_http_header_cannot_carry_is_refused() {
        // A newline would split the request, a space would arrive as a pasted
        // `Bearer ...` prefix, and anything outside visible ASCII cannot be a
        // header value at all. Refusing them here is what lets `credential`
        // be infallible.
        for raw in [
            "ghp_with space",
            "ghp_with\nnewline",
            "ghp_with\ttab",
            "ghp_with\u{7f}delete",
            "ghp_with\u{e9}accent",
        ] {
            assert!(
                matches!(Token::parse(raw), Err(GitHubError::UnusableToken)),
                "expected {raw:?} to be refused as unusable"
            );
        }
    }

    #[test]
    fn surrounding_whitespace_is_trimmed_rather_than_refused() {
        // A token read from a file or pasted from a terminal routinely arrives
        // with a trailing newline. Refusing it would report as "GitHub rejected
        // the token", which sends the reader looking in the wrong place.
        for raw in [SECRET, &format!("{SECRET}\n"), &format!("  {SECRET}\t ")] {
            let credential = Token::parse(raw).as_ref().map(Token::credential).ok();
            assert_eq!(
                credential.as_ref().and_then(|value| value.to_str().ok()),
                Some(format!("bearer {SECRET}").as_str()),
                "expected {raw:?} to normalise to the bare token"
            );
        }
    }

    #[test]
    fn the_credential_is_marked_sensitive_so_headers_do_not_print_it() {
        let credential = Token::parse(SECRET).as_ref().map(Token::credential).ok();
        assert_eq!(
            credential.as_ref().map(HeaderValue::is_sensitive),
            Some(true),
            "an unmarked header value prints in full wherever reqwest debugs its headers"
        );
        assert!(
            !format!("{credential:?}").contains(SECRET),
            "a sensitive header value must debug as a placeholder"
        );
    }

    #[test]
    fn gh_token_wins_over_github_token() {
        // The `gh` CLI documents this precedence. A developer who set both and
        // is authenticated in `gh` expects the same account here.
        let both = lookup(&[(GH_TOKEN, SECRET), (GITHUB_TOKEN, "ghp_the_other_one")]);
        let credential = Token::from_lookup(both)
            .as_ref()
            .map(Token::credential)
            .ok();
        assert_eq!(
            credential.as_ref().and_then(|value| value.to_str().ok()),
            Some(format!("bearer {SECRET}").as_str())
        );
    }

    #[test]
    fn github_token_is_used_when_gh_token_is_absent_or_blank() {
        for pairs in [
            vec![(GITHUB_TOKEN, SECRET)],
            vec![(GH_TOKEN, "  "), (GITHUB_TOKEN, SECRET)],
        ] {
            let credential = Token::from_lookup(lookup(&pairs))
                .as_ref()
                .map(Token::credential)
                .ok();
            assert_eq!(
                credential.as_ref().and_then(|value| value.to_str().ok()),
                Some(format!("bearer {SECRET}").as_str()),
                "expected {pairs:?} to fall through to {GITHUB_TOKEN}"
            );
        }
    }

    #[test]
    fn no_token_anywhere_is_reported_as_missing_not_as_a_rejection() {
        // The two are a different instruction to the reader: one says "set a
        // variable", the other says "your token is wrong".
        assert!(matches!(
            Token::from_lookup(lookup(&[])),
            Err(GitHubError::MissingToken)
        ));
        assert!(matches!(
            Token::from_lookup(lookup(&[("SOMETHING_ELSE", SECRET)])),
            Err(GitHubError::MissingToken)
        ));
    }

    #[test]
    fn a_token_from_the_environment_that_cannot_be_sent_is_not_reported_as_missing() {
        assert!(matches!(
            Token::from_lookup(lookup(&[(GH_TOKEN, "ghp_with space")])),
            Err(GitHubError::UnusableToken)
        ));
    }

    #[test]
    fn no_error_this_module_produces_mentions_the_token() {
        let leaky = format!("{SECRET}\u{7f}");
        let errors: Vec<GitHubError> = [
            Token::parse("").err(),
            Token::parse(&leaky).err(),
            Token::from_lookup(lookup(&[(GH_TOKEN, leaky.as_str())])).err(),
        ]
        .into_iter()
        .flatten()
        .collect();
        assert_eq!(
            errors.len(),
            3,
            "every case here must fail or this test proves nothing"
        );

        for error in errors {
            assert!(
                !format!("{error}").contains(SECRET),
                "an error message must never carry the credential: {error}"
            );
            assert!(
                !format!("{error:?}").contains(SECRET),
                "a Debug rendering must never carry the credential: {error:?}"
            );
        }
    }
}
