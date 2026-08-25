//! The domain vocabulary: checked newtypes over the primitives GitHub hands us.

use std::hash::{Hash, Hasher};
use std::num::NonZeroU32;

/// The length of a full SHA-1 object id in hex, which is what `headRefOid`
/// returns. Named for SHA-1 so that widening to SHA-256's 64 is an obvious
/// edit rather than a silent reinterpretation of a magic number.
const SHA1_HEX_LEN: usize = 40;

/// Trims `raw`, yielding `None` if nothing is left.
///
/// Every value in this module whose only rule is "not blank" normalises through
/// here, so they cannot drift apart in what counts as whitespace. The caller
/// supplies the error, because which blank it was is the caller's business.
fn trimmed_non_empty(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// Splits the text of a GitHub API enum value into the spelling worth keeping
/// and the key to match it on.
///
/// Every enum in this crate that reads a wire value goes through here, so they
/// cannot drift apart in what counts as the same value — a second copy of this
/// rule is a second thing to keep in step, and the tests would not notice if
/// the two stopped agreeing.
///
/// Surrounding whitespace is stripped and ASCII case is folded, because an API
/// enum's values are unique whatever their case: reading `FAILURE` and
/// `failure` as two states would report a failure as something nobody has seen
/// before. That is the opposite of [`CheckName`], where case is meaning,
/// because GitHub matches a required context against a reported name literally.
///
/// The trimmed spelling is returned *alongside* the key rather than derived
/// from it, so an unrecognised value can be preserved exactly as it arrived
/// rather than in the upper case this function invented.
pub(crate) fn wire_value(raw: &str) -> (&str, String) {
    let trimmed = raw.trim();
    (trimmed, trimmed.to_ascii_uppercase())
}

/// Feeds `value` to `state` as if it were lowercased, without allocating.
///
/// Paired with [`str::eq_ignore_ascii_case`] to give a type case-insensitive
/// identity. Both sides of that pair must fold the same way or a `HashMap`
/// lookup misses an entry that compares equal, so every case-insensitive type
/// in this module hashes through here rather than rolling its own loop.
///
/// The trailing `0xff` mirrors what `impl Hash for str` writes: without a
/// terminator, hashing two fields in sequence would let `("ab", "c")` and
/// `("a", "bc")` collide.
fn hash_ascii_lowercase<H: Hasher>(value: &str, state: &mut H) {
    for byte in value.bytes() {
        state.write_u8(byte.to_ascii_lowercase());
    }
    state.write_u8(0xff);
}

/// Why a domain value could not be constructed.
///
/// Every checked constructor in this crate reports failure here, so a caller
/// converting a whole payload only has to handle one error type. Each variant
/// names a single, distinct way the input was wrong, so a caller can react to
/// one cause without string-matching a message.
///
/// No variant carries the offending text. A rejected value is by definition
/// untrusted input, and this crate does not decide how a shell should quote it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum InvalidValue {
    /// A pull request number outside the range GitHub assigns.
    ///
    /// Reached from two directions. [`PrNumber::new`] rejects zero, the only
    /// `u32` GitHub never assigns; [`TryFrom<i64>`] rejects a negative or a
    /// value past `u32::MAX`, which the wire can carry because GraphQL's `Int`
    /// is signed.
    ///
    /// The message names the whole range for that reason. Reporting a negative
    /// as "must be greater than zero" would send the reader looking for a zero
    /// that was never in the payload.
    ///
    /// [`TryFrom<i64>`]: PrNumber#impl-TryFrom<i64>-for-PrNumber
    #[error("a pull request number must be between 1 and {}", u32::MAX)]
    PrNumberOutOfRange,

    /// A login that was empty, or held nothing but whitespace.
    #[error("a login must not be blank")]
    BlankLogin,

    /// A check name that was empty, or held nothing but whitespace.
    #[error("a check name must not be blank")]
    BlankCheckName,

    /// An object id that was not a full SHA-1, including any abbreviation.
    ///
    /// The `len` is a count of characters. It is only reached once every
    /// character is known to be a single-byte lowercase hex digit.
    #[error(
        "an object id must be exactly {} characters long, got {len}",
        SHA1_HEX_LEN
    )]
    HeadShaNotFullSha1 {
        /// How many characters were supplied.
        len: usize,
    },

    /// An object id containing something other than `0`-`9` or `a`-`f`.
    ///
    /// Uppercase is rejected rather than folded: git writes object ids in
    /// lowercase, so uppercase means the value came from somewhere unexpected.
    #[error("an object id must contain only lowercase hex digits")]
    HeadShaNotLowercaseHex,

    /// A repository reference that was not two halves around one slash.
    ///
    /// Covers both too few slashes (`octocat`) and too many
    /// (`github.com/octocat/hello-world`).
    #[error("a repository must be written as \"owner/name\"")]
    RepoRefNotOwnerSlashName,

    /// A repository reference whose owner half was blank, as in `/hello-world`.
    #[error("a repository owner must not be blank")]
    BlankRepoOwner,

    /// A repository reference whose name half was blank, as in `octocat/`.
    #[error("a repository name must not be blank")]
    BlankRepoName,
}

/// A pull request number, unique within its repository.
///
/// This is the number in the URL and the one people say out loud — `#4225`, not
/// the opaque node id. It exists as a type so that a number cannot be passed
/// where a count, an approval total, or another repository's number is meant.
///
/// The invariant is structural rather than checked: the wrapped
/// [`NonZeroU32`] cannot hold zero, so no code path inside this crate can
/// forge an out-of-range number either. That also makes `Option<PrNumber>` the
/// same size as `PrNumber`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PrNumber(NonZeroU32);

impl PrNumber {
    /// Wraps a raw pull request number after checking it is one GitHub could
    /// have assigned.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidValue::PrNumberOutOfRange`] for zero, which is the only
    /// `u32` GitHub never assigns.
    ///
    /// # Examples
    ///
    /// ```
    /// use prtrack_core::PrNumber;
    ///
    /// let pr = PrNumber::new(4225)?;
    /// assert_eq!(pr.get(), 4225);
    /// assert!(PrNumber::new(0).is_err());
    /// # Ok::<(), prtrack_core::InvalidValue>(())
    /// ```
    pub fn new(value: u32) -> Result<Self, InvalidValue> {
        NonZeroU32::new(value)
            .map(Self)
            .ok_or(InvalidValue::PrNumberOutOfRange)
    }

    /// The number as GitHub writes it, for rendering and for building URLs.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

impl TryFrom<i64> for PrNumber {
    type Error = InvalidValue;

    /// Reads a pull request number as the GraphQL API reports it.
    ///
    /// GraphQL's `Int` is signed, so the wire can carry a negative and — in
    /// principle — a value past `u32::MAX`. Both are rejected here rather than
    /// at each call site: a boundary that has to reconcile [`TryFromIntError`]
    /// with [`InvalidValue`] itself will do it differently in each place, and
    /// one of those places will get it wrong.
    ///
    /// [`TryFromIntError`]: std::num::TryFromIntError
    ///
    /// # Errors
    ///
    /// Returns [`InvalidValue::PrNumberOutOfRange`] for anything outside
    /// `1..=u32::MAX`, which is every value GitHub never assigns.
    ///
    /// # Examples
    ///
    /// ```
    /// use prtrack_core::{InvalidValue, PrNumber};
    ///
    /// assert_eq!(PrNumber::try_from(4225_i64)?.get(), 4225);
    /// assert_eq!(PrNumber::try_from(-1_i64), Err(InvalidValue::PrNumberOutOfRange));
    /// assert_eq!(PrNumber::try_from(0_i64), Err(InvalidValue::PrNumberOutOfRange));
    /// # Ok::<(), prtrack_core::InvalidValue>(())
    /// ```
    fn try_from(value: i64) -> Result<Self, Self::Error> {
        u32::try_from(value)
            .map_err(|_source| InvalidValue::PrNumberOutOfRange)
            .and_then(Self::new)
    }
}

/// The handle of a GitHub account — an author, a reviewer, an assignee.
///
/// Logins are the identity the whole board is addressed to: whose review is
/// being waited on, who requested changes, whether a pull request is yours.
/// Keeping them out of `String` is what stops a login being rendered into a
/// column that wanted a check name, and stops a check name being compared
/// against a reviewer.
///
/// Surrounding whitespace is stripped on construction, so equality and hashing
/// compare the login itself and not an artefact of how it was quoted.
///
/// # Identity ignores case
///
/// GitHub account names are case-insensitive: `users/OCTOCAT` resolves to
/// `octocat`, and two accounts differing only in case cannot exist. So
/// `PartialEq`, `Eq`, and `Hash` fold ASCII case, while [`Login::as_str`]
/// returns the spelling this value was built from. Display keeps what the API
/// or the user wrote; identity does not depend on it.
///
/// Two logins can therefore be equal and still render differently. Do not
/// deduplicate by rendered text, and do not assume `a == b` implies
/// `a.as_str() == b.as_str()`.
///
/// A login is deliberately not interchangeable with a [`HeadSha`], even though
/// nothing about the characters in one rules out the other. The well-typed
/// call compiles:
///
/// ```
/// use prtrack_core::{HeadSha, Login};
///
/// fn pushed(_: &HeadSha) {}
///
/// pushed(&HeadSha::parse("9f2c4e1b7a3d5068c9e4b2a17f36d8095c1e4a7b").unwrap());
/// let _login = Login::parse("9f2c4e1b7a3d5068c9e4b2a17f36d8095c1e4a7b").unwrap();
/// ```
///
/// Handing the same function a login does not. Only that one argument differs
/// between the blocks, and every name the failing block depends on appears in
/// the passing one — so a stale import or a renamed constructor reddens the
/// block above instead of leaving the block below green for a reason that has
/// nothing to do with types:
///
/// ```compile_fail,E0308
/// use prtrack_core::{HeadSha, Login};
///
/// fn pushed(_: &HeadSha) {}
///
/// pushed(&Login::parse("9f2c4e1b7a3d5068c9e4b2a17f36d8095c1e4a7b").unwrap());
/// ```
#[derive(Debug, Clone)]
pub struct Login(String);

impl PartialEq for Login {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq_ignore_ascii_case(&other.0)
    }
}

impl Eq for Login {}

impl Hash for Login {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_ascii_lowercase(&self.0, state);
    }
}

impl Login {
    /// Reads a login from raw text, trimming whitespace around it.
    ///
    /// Nothing beyond blankness is enforced. GitHub's own account names include
    /// shapes that look like punctuation errors — `dependabot[bot]` is a real
    /// login — and inventing a stricter character class here would reject
    /// accounts that exist.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidValue::BlankLogin`] when nothing but whitespace was
    /// supplied. An empty login is never a real account, and letting one
    /// through would render as a blank cell that looks like "nobody".
    ///
    /// # Examples
    ///
    /// ```
    /// use prtrack_core::Login;
    ///
    /// let login = Login::parse("  octocat\n")?;
    /// assert_eq!(login.as_str(), "octocat");
    /// assert!(Login::parse("   ").is_err());
    /// # Ok::<(), prtrack_core::InvalidValue>(())
    /// ```
    pub fn parse(raw: &str) -> Result<Self, InvalidValue> {
        trimmed_non_empty(raw)
            .map(|login| Self(login.to_owned()))
            .ok_or(InvalidValue::BlankLogin)
    }

    /// The trimmed login, for rendering and for comparing against the viewer.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The name of a CI check, as the check reports it — `Preview CI Gate`,
/// `build / test (ubuntu-latest)`, `ci/circleci: build`.
///
/// This is the join key of the whole policy decision: a failing check blocks a
/// pull request only when its name appears in the repository's required list.
/// That comparison is the one place a typo would silently turn a blocking
/// failure into an advisory one, so the two sides of it share a type instead of
/// both being `String`.
///
/// Surrounding whitespace is stripped on construction so that a name from a
/// protection rule and the same name from a check run compare equal.
///
/// Unlike [`Login`] and [`RepoRef`], identity here **is** case-sensitive, and
/// deliberately so: GitHub matches a required check context against a reported
/// check name literally, so `Preview CI Gate` and `preview ci gate` are two
/// different checks. Folding case to match its neighbours would classify a
/// failing required check as advisory.
///
/// A check name is deliberately not interchangeable with a [`Login`]. The
/// well-typed call compiles:
///
/// ```
/// use prtrack_core::{CheckName, Login};
///
/// fn required(_: &CheckName) -> bool { true }
///
/// required(&CheckName::parse("octocat").unwrap());
/// let _login = Login::parse("octocat").unwrap();
/// ```
///
/// Handing the same function a login does not, and only that argument differs
/// between the two blocks:
///
/// ```compile_fail,E0308
/// use prtrack_core::{CheckName, Login};
///
/// fn required(_: &CheckName) -> bool { true }
///
/// required(&Login::parse("octocat").unwrap());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CheckName(String);

impl CheckName {
    /// Reads a check name from raw text, trimming whitespace around it.
    ///
    /// Beyond blankness nothing is enforced. Check names are free text chosen by
    /// whoever wrote the workflow, and they routinely contain spaces, slashes,
    /// colons, and parentheses.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidValue::BlankCheckName`] when nothing but whitespace was
    /// supplied. A blank name could never match a required check, so accepting
    /// one would quietly classify a real failure as advisory.
    ///
    /// # Examples
    ///
    /// ```
    /// use prtrack_core::CheckName;
    ///
    /// let check = CheckName::parse("  Preview CI Gate ")?;
    /// assert_eq!(check.as_str(), "Preview CI Gate");
    /// assert!(CheckName::parse("\t").is_err());
    /// # Ok::<(), prtrack_core::InvalidValue>(())
    /// ```
    pub fn parse(raw: &str) -> Result<Self, InvalidValue> {
        trimmed_non_empty(raw)
            .map(|name| Self(name.to_owned()))
            .ok_or(InvalidValue::BlankCheckName)
    }

    /// The trimmed name, for rendering and for matching against a required list.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The git object id at the head of a pull request's branch.
///
/// The head sha is what makes a fact about a pull request checkable: a review,
/// a check run, and a merge status are all statements about one commit, and a
/// push replaces it. Anything cached against the wrong sha is stale state
/// wearing a fresh face, which is exactly the failure this tool exists to
/// avoid — so the sha is a type, not a string that happens to be nearby.
///
/// The only accepted value is a full 40-character lowercase hex SHA-1, exactly
/// as `headRefOid` returns it. Abbreviations are rejected rather than accepted
/// and compared loosely: this type compares textually, so an abbreviation held
/// against the full id it abbreviates would report a push that never happened —
/// a false alarm, which costs more here than a missed one.
///
/// Should GitHub move to SHA-256, widening this to accept 64 characters as well
/// is a non-breaking change. Narrowing later would not be, which is why it
/// starts strict.
///
/// An object id is deliberately not interchangeable with a [`CheckName`], even
/// though a check could plausibly be named after a commit. The well-typed call
/// compiles:
///
/// ```
/// use prtrack_core::{CheckName, HeadSha};
///
/// fn at_commit(_: &HeadSha) {}
///
/// at_commit(&HeadSha::parse("9f2c4e1b7a3d5068c9e4b2a17f36d8095c1e4a7b").unwrap());
/// let _name = CheckName::parse("9f2c4e1b7a3d5068c9e4b2a17f36d8095c1e4a7b").unwrap();
/// ```
///
/// Handing the same function a check name does not, and only that argument
/// differs between the two blocks:
///
/// ```compile_fail,E0308
/// use prtrack_core::{CheckName, HeadSha};
///
/// fn at_commit(_: &HeadSha) {}
///
/// at_commit(&CheckName::parse("9f2c4e1b7a3d5068c9e4b2a17f36d8095c1e4a7b").unwrap());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HeadSha(String);

impl HeadSha {
    /// Reads an object id from raw text.
    ///
    /// Unlike [`Login::parse`] and [`CheckName::parse`], the input is *not*
    /// trimmed. Whitespace is not part of an object id, so a padded value means
    /// the caller sliced something wrong, and repairing it here would hide
    /// that.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidValue::HeadShaNotLowercaseHex`] if any character is
    /// outside `0`-`9` and `a`-`f`, which includes uppercase hex and
    /// surrounding whitespace. Otherwise returns
    /// [`InvalidValue::HeadShaNotFullSha1`] unless the id is exactly 40
    /// characters. The alphabet is checked first, so the length a failure
    /// reports is always a true character count.
    ///
    /// # Examples
    ///
    /// ```
    /// use prtrack_core::HeadSha;
    ///
    /// let sha = HeadSha::parse("9f2c4e1b7a3d5068c9e4b2a17f36d8095c1e4a7b")?;
    /// assert_eq!(sha.as_str().len(), 40);
    ///
    /// assert!(HeadSha::parse("9f2c4e1").is_err()); // abbreviated
    /// assert!(HeadSha::parse("9F2C4E1B7A3D5068C9E4B2A17F36D8095C1E4A7B").is_err()); // uppercase
    /// # Ok::<(), prtrack_core::InvalidValue>(())
    /// ```
    pub fn parse(raw: &str) -> Result<Self, InvalidValue> {
        if !raw
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(InvalidValue::HeadShaNotLowercaseHex);
        }
        let len = raw.len();
        if len != SHA1_HEX_LEN {
            return Err(InvalidValue::HeadShaNotFullSha1 { len });
        }
        Ok(Self(raw.to_owned()))
    }

    /// The object id, for rendering and for comparing one snapshot to the next.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The repository a pull request belongs to, as `owner/name`.
///
/// This is the unit everything else is scoped by: a [`PrNumber`] only means
/// something alongside the repository that issued it, and a policy read from
/// one repository must never be applied to another's pull requests. Parsing
/// `owner/name` once, here, keeps that split out of every call site that would
/// otherwise re-derive it from a string.
///
/// Beyond "two non-blank halves around one slash" no character rules are
/// enforced. GitHub's own naming rules have changed over time and a stricter
/// guess here would reject repositories that exist; a name that GitHub does not
/// recognise fails at the API call with a message from GitHub itself.
///
/// # Identity ignores case
///
/// GitHub resolves `repos/example-org/EXAMPLE-REPO` to `Example-Org/example-repo`,
/// and one owner cannot hold two repositories differing only in case. Both
/// halves therefore fold ASCII case for `PartialEq`, `Eq`, and `Hash`, while
/// [`RepoRef::owner`], [`RepoRef::name`], and `Display` return the spelling
/// this value was built from.
///
/// This matters most where the two sources meet: `--repo` is typed by a human
/// and everything it is matched against arrives from the API in canonical
/// casing. Case-sensitive identity would turn a capitalisation difference into
/// an empty board, which reads as "nothing to show" rather than as a mismatch.
#[derive(Debug, Clone)]
pub struct RepoRef {
    owner: String,
    name: String,
}

impl PartialEq for RepoRef {
    fn eq(&self, other: &Self) -> bool {
        self.owner.eq_ignore_ascii_case(&other.owner) && self.name.eq_ignore_ascii_case(&other.name)
    }
}

impl Eq for RepoRef {}

impl Hash for RepoRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_ascii_lowercase(&self.owner, state);
        hash_ascii_lowercase(&self.name, state);
    }
}

impl RepoRef {
    /// Splits an `owner/name` reference, trimming whitespace around each half.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidValue::RepoRefNotOwnerSlashName`] unless there is
    /// exactly one slash, then [`InvalidValue::BlankRepoOwner`] or
    /// [`InvalidValue::BlankRepoName`] if the corresponding half is blank once
    /// trimmed. The owner is checked first, so `"/"` reports a blank owner.
    ///
    /// # Examples
    ///
    /// ```
    /// use prtrack_core::RepoRef;
    ///
    /// let repo = RepoRef::parse(" octocat/hello-world ")?;
    /// assert_eq!(repo.owner(), "octocat");
    /// assert_eq!(repo.name(), "hello-world");
    /// assert_eq!(repo.to_string(), "octocat/hello-world");
    ///
    /// assert!(RepoRef::parse("hello-world").is_err());
    /// assert!(RepoRef::parse("github.com/octocat/hello-world").is_err());
    /// # Ok::<(), prtrack_core::InvalidValue>(())
    /// ```
    pub fn parse(raw: &str) -> Result<Self, InvalidValue> {
        let (owner, name) = raw
            .split_once('/')
            .ok_or(InvalidValue::RepoRefNotOwnerSlashName)?;
        if name.contains('/') {
            return Err(InvalidValue::RepoRefNotOwnerSlashName);
        }

        let owner = trimmed_non_empty(owner).ok_or(InvalidValue::BlankRepoOwner)?;
        let name = trimmed_non_empty(name).ok_or(InvalidValue::BlankRepoName)?;

        Ok(Self {
            owner: owner.to_owned(),
            name: name.to_owned(),
        })
    }

    /// The account that owns the repository — a user or an organisation.
    #[must_use]
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// The repository name, without its owner.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl std::fmt::Display for RepoRef {
    /// Renders the canonical `owner/name` form, which round-trips through
    /// [`RepoRef::parse`].
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::{CheckName, HeadSha, InvalidValue, Login, PrNumber, RepoRef};
    use std::collections::hash_map::DefaultHasher;
    use std::collections::HashMap;
    use std::hash::{Hash, Hasher};

    /// The hash a value would get in a `HashMap`, so a test can pin that `Hash`
    /// agrees with `Eq` instead of trusting that a hand-written pair matches.
    fn hash_of<T: Hash>(value: &T) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn login_identity_ignores_case_because_github_accounts_do() {
        // Compared as `Option`, with a guard: two `Err`s are equal to each
        // other, so comparing the `Result`s directly would pass if construction
        // broke entirely.
        let lower = Login::parse("octocat").ok();
        assert!(
            lower.is_some(),
            "both sides must parse or this proves nothing"
        );
        assert_eq!(lower, Login::parse("OctoCat").ok());
        assert_eq!(lower, Login::parse("OCTOCAT").ok());
    }

    #[test]
    fn login_keeps_the_spelling_it_was_given_for_display() {
        assert_eq!(
            Login::parse("OctoCat").as_ref().map(Login::as_str),
            Ok("OctoCat")
        );
    }

    #[test]
    fn logins_that_compare_equal_hash_equally() {
        let lower = Login::parse("octocat").ok();
        let mixed = Login::parse("OctoCat").ok();
        assert!(
            lower.is_some(),
            "both sides must parse or this proves nothing"
        );
        assert_eq!(lower, mixed);
        assert_eq!(hash_of(&lower), hash_of(&mixed));
    }

    #[test]
    fn repo_ref_identity_ignores_case_in_both_halves() {
        let canonical = RepoRef::parse("Example-Org/example-repo").ok();
        assert!(
            canonical.is_some(),
            "both sides must parse or this proves nothing"
        );
        assert_eq!(canonical, RepoRef::parse("example-org/EXAMPLE-REPO").ok());
    }

    #[test]
    fn repo_ref_keeps_the_spelling_it_was_given_for_display() {
        assert_eq!(
            RepoRef::parse("Example-Org/example-repo").map(|repo| repo.to_string()),
            Ok("Example-Org/example-repo".to_owned())
        );
    }

    #[test]
    fn repo_refs_that_compare_equal_hash_equally() {
        let canonical = RepoRef::parse("Example-Org/example-repo").ok();
        let typed = RepoRef::parse("example-org/EXAMPLE-REPO").ok();
        assert!(
            canonical.is_some(),
            "both sides must parse or this proves nothing"
        );
        assert_eq!(canonical, typed);
        assert_eq!(hash_of(&canonical), hash_of(&typed));
    }

    #[test]
    fn a_repo_typed_in_the_wrong_case_still_finds_its_entry() {
        // The board is keyed by the repository the API named; the user types
        // `--repo` by hand. A miss here renders as an empty board, which reads
        // as "nothing to show" rather than "you typed a different case".
        let canonical = RepoRef::parse("Example-Org/example-repo").ok();
        let typed = RepoRef::parse("example-org/EXAMPLE-REPO").ok();
        assert!(
            canonical.is_some() && typed.is_some(),
            "both sides must parse or this proves nothing"
        );

        let mut boards = HashMap::new();
        boards.insert(canonical, 37_u32);

        assert_eq!(boards.get(&typed), Some(&37));
    }

    #[test]
    fn check_name_identity_is_case_sensitive_because_github_matches_it_literally() {
        assert_ne!(
            CheckName::parse("Preview CI Gate"),
            CheckName::parse("preview ci gate")
        );
    }

    #[test]
    fn pr_number_accepts_any_number_github_can_assign() {
        for value in [1_u32, 42, 4225, u32::MAX] {
            assert_eq!(
                PrNumber::new(value).map(PrNumber::get),
                Ok(value),
                "expected {value} to be a valid pull request number"
            );
        }
    }

    #[test]
    fn a_wire_number_github_could_have_assigned_converts() {
        // GraphQL reports the number as a signed `Int`, so the boundary hands
        // this an `i64` and every value in range must survive the narrowing.
        for value in [1_i64, 4225, 4354, i64::from(u32::MAX)] {
            assert_eq!(
                PrNumber::try_from(value).map(PrNumber::get),
                u32::try_from(value).map_err(|_| InvalidValue::PrNumberOutOfRange),
                "expected {value} to convert to itself"
            );
        }
    }

    #[test]
    fn a_wire_number_outside_the_range_github_assigns_is_refused() {
        // A negative reaching `PrNumber::new` as a `u32` would be a different
        // pull request entirely: -1 narrows to 4294967295, which is a number
        // some repository could genuinely have.
        for value in [
            -1_i64,
            0,
            -4354,
            i64::MIN,
            i64::from(u32::MAX) + 1,
            i64::MAX,
        ] {
            assert_eq!(
                PrNumber::try_from(value),
                Err(InvalidValue::PrNumberOutOfRange),
                "expected {value} to be refused as out of range"
            );
        }
    }

    #[test]
    fn the_out_of_range_message_does_not_claim_the_value_was_zero() {
        // The variant covers negatives and values past `u32::MAX` as well, and
        // reporting one of those as "must be greater than zero" sends the
        // reader looking for a zero that was never there.
        let rendered = InvalidValue::PrNumberOutOfRange.to_string();
        assert!(
            !rendered.contains("greater than zero"),
            "the message must describe the range, not just its lower end: {rendered}"
        );
        assert!(
            rendered.contains(&u32::MAX.to_string()),
            "the message should name the upper bound it is enforcing: {rendered}"
        );
    }

    #[test]
    fn pr_number_rejects_zero_because_github_numbers_from_one() {
        assert_eq!(PrNumber::new(0), Err(InvalidValue::PrNumberOutOfRange));
    }

    #[test]
    fn login_trims_surrounding_whitespace() {
        for raw in ["octocat", " octocat", "octocat\n", "  \toctocat \r\n"] {
            assert_eq!(
                Login::parse(raw).as_ref().map(Login::as_str),
                Ok("octocat"),
                "expected {raw:?} to normalise to the bare login"
            );
        }
    }

    #[test]
    fn login_keeps_the_punctuation_github_puts_in_bot_and_org_logins() {
        for raw in ["dependabot[bot]", "some-org", "a"] {
            assert_eq!(Login::parse(raw).as_ref().map(Login::as_str), Ok(raw));
        }
    }

    #[test]
    fn login_rejects_input_that_is_blank_once_trimmed() {
        for raw in ["", " ", "\t", "\n", "   \t\r\n  "] {
            assert_eq!(
                Login::parse(raw),
                Err(InvalidValue::BlankLogin),
                "expected {raw:?} to be rejected as blank"
            );
        }
    }

    #[test]
    fn check_name_trims_surrounding_whitespace() {
        for raw in ["Preview CI Gate", "  Preview CI Gate", "Preview CI Gate\n"] {
            assert_eq!(
                CheckName::parse(raw).as_ref().map(CheckName::as_str),
                Ok("Preview CI Gate"),
                "expected {raw:?} to normalise to the bare check name"
            );
        }
    }

    #[test]
    fn check_name_keeps_the_inner_shape_ci_providers_use() {
        for raw in [
            "build / test (ubuntu-latest)",
            "ci/circleci: build",
            "Preview CI Gate",
        ] {
            assert_eq!(
                CheckName::parse(raw).as_ref().map(CheckName::as_str),
                Ok(raw)
            );
        }
    }

    #[test]
    fn check_name_rejects_input_that_is_blank_once_trimmed() {
        for raw in ["", " ", "\t", "  \r\n "] {
            assert_eq!(
                CheckName::parse(raw),
                Err(InvalidValue::BlankCheckName),
                "expected {raw:?} to be rejected as blank"
            );
        }
    }

    /// A full 40-character object id, as `headRefOid` returns it.
    const FULL_SHA: &str = "9f2c4e1b7a3d5068c9e4b2a17f36d8095c1e4a7b";

    #[test]
    fn head_sha_accepts_a_full_object_id() {
        assert_eq!(
            HeadSha::parse(FULL_SHA).as_ref().map(HeadSha::as_str),
            Ok(FULL_SHA)
        );
    }

    #[test]
    fn head_sha_rejects_abbreviations_because_it_is_compared_textually() {
        // An abbreviation compared against a full id reports a push that never
        // happened. Nothing in the wire format produces one: `headRefOid` is
        // always full length.
        for len in [1_usize, 6, 7, 8, 12, 39] {
            let raw = &FULL_SHA[..len];
            assert_eq!(
                HeadSha::parse(raw),
                Err(InvalidValue::HeadShaNotFullSha1 { len }),
                "expected the {len}-character abbreviation {raw:?} to be rejected"
            );
        }
    }

    #[test]
    fn head_sha_rejects_lengths_either_side_of_a_full_object_id() {
        let too_long = format!("{FULL_SHA}0");
        let cases: [(&str, usize); 3] = [("", 0), (&FULL_SHA[..39], 39), (too_long.as_str(), 41)];
        for (raw, len) in cases {
            assert_eq!(
                HeadSha::parse(raw),
                Err(InvalidValue::HeadShaNotFullSha1 { len }),
                "expected {raw:?} to be rejected for its length"
            );
        }
    }

    #[test]
    fn repo_ref_splits_on_the_single_slash() {
        for (raw, owner, name) in [
            ("Example-Org/example-repo", "Example-Org", "example-repo"),
            ("strigidata/gh-pr-tracker", "strigidata", "gh-pr-tracker"),
            ("owner/name.with.dots", "owner", "name.with.dots"),
        ] {
            let parsed = RepoRef::parse(raw);
            assert_eq!(
                parsed.as_ref().map(RepoRef::owner),
                Ok(owner),
                "wrong owner for {raw:?}"
            );
            assert_eq!(
                parsed.as_ref().map(RepoRef::name),
                Ok(name),
                "wrong name for {raw:?}"
            );
        }
    }

    #[test]
    fn repo_ref_trims_whitespace_around_each_half() {
        for raw in [
            "octocat/hello-world",
            "  octocat/hello-world  ",
            "octocat / hello-world",
            "\toctocat/hello-world\n",
        ] {
            assert_eq!(
                RepoRef::parse(raw).as_ref().map(RepoRef::owner),
                Ok("octocat"),
                "wrong owner for {raw:?}"
            );
            assert_eq!(
                RepoRef::parse(raw).as_ref().map(RepoRef::name),
                Ok("hello-world"),
                "wrong name for {raw:?}"
            );
        }
    }

    #[test]
    fn repo_ref_renders_back_to_the_form_it_was_parsed_from() {
        assert_eq!(
            RepoRef::parse("  octocat / hello-world ").map(|repo| repo.to_string()),
            Ok("octocat/hello-world".to_owned())
        );
    }

    #[test]
    fn repo_ref_rejects_anything_that_is_not_exactly_one_slash() {
        for raw in ["", "noslash", "a/b/c", "owner//name", "///"] {
            assert_eq!(
                RepoRef::parse(raw),
                Err(InvalidValue::RepoRefNotOwnerSlashName),
                "expected {raw:?} to be rejected for its shape"
            );
        }
    }

    #[test]
    fn repo_ref_rejects_a_blank_owner_before_it_looks_at_the_name() {
        for raw in ["/name", "   /name", "/", " / "] {
            assert_eq!(
                RepoRef::parse(raw),
                Err(InvalidValue::BlankRepoOwner),
                "expected {raw:?} to be rejected for its owner"
            );
        }
    }

    #[test]
    fn repo_ref_rejects_a_blank_name() {
        for raw in ["owner/", "owner/   ", "  owner  /  \t "] {
            assert_eq!(
                RepoRef::parse(raw),
                Err(InvalidValue::BlankRepoName),
                "expected {raw:?} to be rejected for its name"
            );
        }
    }

    #[test]
    fn head_sha_rejects_anything_that_is_not_a_lowercase_hex_digit() {
        for raw in [
            "9F2C4E1",                                  // uppercase
            "9f2C4e1",                                  // mixed case
            "ghijklm",                                  // outside the hex alphabet
            "9f2c4e1 ",                                 // trailing space: not repaired
            " 9f2c4e1",                                 // leading space: not repaired
            "9f2c-4e1",                                 // punctuation
            "9f2c4e1b7a3d5068c9e4b2a17f36d8095c1e4a7X", // full length, one bad digit
        ] {
            assert_eq!(
                HeadSha::parse(raw),
                Err(InvalidValue::HeadShaNotLowercaseHex),
                "expected {raw:?} to be rejected as non-hex"
            );
        }
    }
}
