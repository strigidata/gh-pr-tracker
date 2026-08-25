//! Noticing that a newer `prtrack` exists, without getting in the way.
//!
//! This is a courtesy, and courtesies must be cheap and silent. A version check
//! that costs a request on every run, or that prints a failure when GitHub is
//! unreachable, is worse than no version check at all — the tool would be
//! complaining about itself while the board it exists to draw sits underneath.
//!
//! So: one request an hour at most, the answer cached on disk, and every
//! failure swallowed. The only thing that ever reaches the screen is a single
//! line saying a newer version is out.
//!
//! # Why this works on a private repository
//!
//! `prtrack` cannot do anything without a GitHub token that can read the
//! repository being watched. The same token reads the releases of the
//! repository `prtrack` itself ships from, so a private distribution needs no
//! public endpoint, no second credential and nothing hosted. Whoever can
//! install the tool can see its releases, by construction.

use std::path::Path;

/// Where releases are published. Overridable so a fork, or a test, can point
/// somewhere else without a rebuild.
const RELEASE_REPO: &str = "strigidata/gh-pr-tracker";

/// The environment variable that overrides it.
const RELEASE_REPO_ENV: &str = "PRTRACK_RELEASE_REPO";

/// How long an answer is trusted.
///
/// An hour is far longer than a release takes to appear and far shorter than
/// anybody would stay on a stale build, and it keeps the cost at one request
/// per hour of watching rather than one per poll.
pub const CHECK_INTERVAL_SECONDS: i64 = 60 * 60;

/// The version this binary was built as.
#[must_use]
pub const fn running() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Which repository to read releases from.
#[must_use]
pub fn release_repo() -> String {
    std::env::var(RELEASE_REPO_ENV).unwrap_or_else(|_| RELEASE_REPO.to_owned())
}

/// What a check concluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Update {
    /// A newer release exists.
    Newer {
        /// Its version, without any leading `v`.
        version: String,
    },
    /// This is the newest, or nothing has been published.
    Current,
    /// Not known — never checked, the check failed, or the answer is stale and
    /// has not been refreshed yet.
    ///
    /// Distinct from [`Update::Current`] on purpose. "You are up to date" is a
    /// claim, and a failed request cannot make it.
    Unknown,
}

impl Update {
    /// The line to print, or nothing.
    ///
    /// Only [`Update::Newer`] says anything. A tool that announces being up to
    /// date every time it runs is a tool people stop reading.
    #[must_use]
    pub fn notice(&self) -> Option<String> {
        match self {
            Self::Newer { version } => Some(format!(
                "❗ prtrack {version} is out — you are on {}. \
                 gh release download v{version} --repo {}",
                running(),
                release_repo()
            )),
            Self::Current | Self::Unknown => None,
        }
    }
}

/// What a previous run learned, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cached {
    /// Seconds since the Unix epoch, when the check ran.
    pub checked_at: i64,
    /// The newest tag seen, or `None` where nothing is published yet.
    pub latest: Option<String>,
}

impl Cached {
    /// Whether this answer is still worth trusting at `now`.
    ///
    /// A timestamp in the future counts as stale rather than fresh. A clock
    /// that moved backwards would otherwise pin a cache as valid indefinitely,
    /// and the failure would be invisible.
    #[must_use]
    pub const fn fresh_at(&self, now: i64) -> bool {
        let age = now - self.checked_at;
        age >= 0 && age < CHECK_INTERVAL_SECONDS
    }
}

/// Reads the cached answer, or `None` if there is not a usable one.
///
/// Every failure — absent, unreadable, malformed — is `None`, which simply
/// means the check runs again.
#[must_use]
pub fn read_cache(path: &Path) -> Option<Cached> {
    let raw = std::fs::read_to_string(path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&raw).ok()?;
    Some(Cached {
        checked_at: parsed.get("checked_at")?.as_i64()?,
        latest: parsed
            .get("latest")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned),
    })
}

/// Writes an answer for the next run.
///
/// Best-effort: a cache that cannot be written costs one extra request an hour,
/// which is not worth telling anybody about.
pub fn write_cache(path: &Path, cached: &Cached) {
    let body = serde_json::json!({
        "checked_at": cached.checked_at,
        "latest": cached.latest,
    });
    if let Some(parent) = path.parent() {
        drop(std::fs::create_dir_all(parent));
    }
    drop(std::fs::write(path, body.to_string()));
}

/// Compares a published tag against the running version.
///
/// The tag may carry a leading `v`; the running version never does.
#[must_use]
pub fn compare(running: &str, published: Option<&str>) -> Update {
    let Some(published) = published else {
        return Update::Current;
    };
    let published = published.strip_prefix('v').unwrap_or(published);
    if is_newer(published, running) {
        Update::Newer {
            version: published.to_owned(),
        }
    } else {
        Update::Current
    }
}

/// Whether `candidate` is a later version than `running`.
///
/// Compared field by field as numbers, which is the whole reason this is not a
/// string comparison: `0.1.10` is newer than `0.1.9` and sorts before it as
/// text. A field that is not a number stops the comparison and reports "not
/// newer", so a tag like `v1.0.0-rc1` never nags somebody on a release build.
fn is_newer(candidate: &str, running: &str) -> bool {
    // Both are parsed in full *before* anything is compared. Comparing field by
    // field as it goes looks equivalent and is not: `1.0.0-rc1` against `0.1.0`
    // decides on the leading `1 > 0` and returns before ever reaching the field
    // that is not a number, so a pre-release would nag somebody on a stable
    // build. The unparseable field has to be found first to be respected.
    let (Some(theirs), Some(ours)) = (numeric_fields(candidate), numeric_fields(running)) else {
        return false;
    };

    // Padded rather than requiring equal length, so `0.2` and `0.2.0` are the
    // same version rather than one being mysteriously newer.
    let width = theirs.len().max(ours.len());
    for index in 0..width {
        let theirs_field = theirs.get(index).copied().unwrap_or(0);
        let ours_field = ours.get(index).copied().unwrap_or(0);
        if theirs_field != ours_field {
            return theirs_field > ours_field;
        }
    }
    false
}

/// A version split into numbers, or `None` if any field is not one.
///
/// All-or-nothing on purpose: a partially understood version is the thing that
/// produces a confident wrong answer.
fn numeric_fields(version: &str) -> Option<Vec<u64>> {
    version
        .split('.')
        .map(|field| field.parse::<u64>().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{compare, Cached, Update, CHECK_INTERVAL_SECONDS};

    #[test]
    fn a_later_version_is_offered_and_an_earlier_one_is_not() {
        assert_eq!(
            compare("0.1.0", Some("v0.1.1")),
            Update::Newer {
                version: "0.1.1".to_owned()
            },
            "the leading v is not part of the version"
        );
        assert_eq!(compare("0.1.1", Some("v0.1.1")), Update::Current);
        assert_eq!(
            compare("0.2.0", Some("v0.1.9")),
            Update::Current,
            "a published version older than this build is not an update"
        );
    }

    #[test]
    fn versions_are_compared_as_numbers_not_as_text() {
        // The reason this is not `a < b` on strings. Ten is after nine, and
        // sorts before it as text -- so a string comparison would go quiet at
        // exactly the point a project starts shipping regularly.
        assert_eq!(
            compare("0.1.9", Some("v0.1.10")),
            Update::Newer {
                version: "0.1.10".to_owned()
            },
            "0.1.10 is newer than 0.1.9"
        );
        assert_eq!(compare("0.1.10", Some("v0.1.9")), Update::Current);
    }

    #[test]
    fn a_tag_that_is_not_a_plain_version_never_nags() {
        // A pre-release or a differently-shaped tag stops the comparison rather
        // than guessing. Nagging somebody on a stable build to "upgrade" to an
        // rc is worse than saying nothing.
        for odd in ["v1.0.0-rc1", "nightly", "v", "1.0.0.beta"] {
            assert_eq!(compare("0.1.0", Some(odd)), Update::Current, "{odd}");
        }
    }

    #[test]
    fn nothing_published_is_not_an_update() {
        assert_eq!(compare("0.1.0", None), Update::Current);
    }

    #[test]
    fn only_a_newer_version_says_anything() {
        // A line printed every run is a line nobody reads, which would blunt
        // the one case that matters.
        assert_eq!(Update::Current.notice(), None);
        assert_eq!(Update::Unknown.notice(), None);

        let notice = Update::Newer {
            version: "9.9.9".to_owned(),
        }
        .notice();
        assert!(
            notice.as_ref().is_some_and(|line| line.contains("9.9.9")),
            "the notice names the version to get"
        );
    }

    #[test]
    fn a_cache_expires_and_a_clock_that_went_backwards_does_not_pin_it() {
        let cached = Cached {
            checked_at: 1_000,
            latest: Some("v0.1.0".to_owned()),
        };
        assert!(cached.fresh_at(1_000), "checked this instant");
        assert!(cached.fresh_at(1_000 + CHECK_INTERVAL_SECONDS - 1));
        assert!(!cached.fresh_at(1_000 + CHECK_INTERVAL_SECONDS), "expired");

        // A stamp in the future means the clock moved, not that the answer is
        // eternally valid. Treating it as fresh would silence the check for
        // however long the skew lasts, with nothing on screen to say so.
        assert!(!cached.fresh_at(999), "a future stamp is stale, not fresh");
    }
}
