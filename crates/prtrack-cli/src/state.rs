//! What the watcher remembers between polls, and how it survives a restart.
//!
//! The file is the difference between a watcher and a poller. Without it every
//! start is a first sighting, which nudges about nothing — so losing it is not
//! a crash, it is a silence, and silence is the failure mode this whole
//! project is built to notice.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use prtrack_core::{
    ApprovalState, HeadSha, InvalidValue, PrNumber, ReasonKind, RequiredCheckHealth, Rereview,
    ReviewRequest, Watched,
};
use serde::{Deserialize, Serialize};

/// The only format this build writes, and the only one it reads.
///
/// Carried in the file so a newer watcher's state is refused by an older one
/// rather than half-understood. Reading an unknown field as absent would mean
/// comparing against a state that was never recorded, which shows up as a
/// nudge that never fires — invisible, and the expensive direction.
const FORMAT_VERSION: u32 = 1;

/// Why the remembered state could not be read or written.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StateError {
    /// The file exists and could not be read or parsed.
    #[error("the watch state at {path} could not be read")]
    Unreadable {
        /// Where it was.
        path: PathBuf,
        /// What went wrong underneath.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// The file was written by a version this build does not understand.
    #[error("the watch state at {path} is version {found}, and this build writes {expected}")]
    UnknownVersion {
        /// Where it was.
        path: PathBuf,
        /// What it claimed to be.
        found: u32,
        /// What this build writes.
        expected: u32,
    },
    /// The file could not be written.
    #[error("the watch state could not be written to {path}")]
    Unwritable {
        /// Where it was going.
        path: PathBuf,
        /// What went wrong underneath.
        #[source]
        source: std::io::Error,
    },
    /// A value in the file is not one the domain accepts.
    #[error("the watch state holds a value this crate cannot read back")]
    Malformed(#[from] InvalidValue),
    /// A reason this build has no name for.
    ///
    /// Its own variant rather than borrowed from [`InvalidValue`]: an unknown
    /// rung means the file was written by a build that knows a rung this one
    /// does not, which is a version problem and not a malformed value.
    #[error("the watch state names a reason this build does not know: {name}")]
    UnknownReason {
        /// What it said.
        name: String,
    },
}

/// One pull request's remembered state, in the shape it takes on disk.
///
/// Separate from [`Watched`] on purpose. That type is the domain's and has
/// private fields; this one is the file format, and the two are allowed to
/// diverge — a field can be added here for diagnostics, or renamed there,
/// without either forcing the other.
#[derive(Debug, Serialize, Deserialize)]
struct StoredPullRequest {
    reason: String,
    head: String,
    review_request: String,
    approval: String,
    required_checks: String,
    /// Absent from a file written before re-reviews were tracked, which reads
    /// as not due: the next push after an upgrade then still nudges, where
    /// refusing the file would have made every pull request a first sighting.
    #[serde(default)]
    rereview: String,
}

/// The file.
#[derive(Debug, Serialize, Deserialize)]
struct StoredState {
    version: u32,
    pull_requests: BTreeMap<u32, StoredPullRequest>,
}

/// Everything the watcher carries from one poll to the next.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WatchState {
    by_number: BTreeMap<u32, Watched>,
}

impl WatchState {
    /// Reads the state a previous run left behind.
    ///
    /// A missing file is an empty state and not an error: the first run on a
    /// machine has none, and refusing to start would make that a failure once
    /// per machine forever.
    ///
    /// An *unreadable* file is an error, though, and deliberately not treated
    /// as empty. The two produce the same behaviour — every pull request looks
    /// new, so nothing nudges — and only one of them is fine. The caller needs
    /// to be able to say which happened.
    ///
    /// # Errors
    ///
    /// [`StateError::Unreadable`] for a file that will not parse,
    /// [`StateError::UnknownVersion`] for one from a newer build, and
    /// [`StateError::Malformed`] for a value the domain refuses.
    pub fn load(path: &Path) -> Result<Self, StateError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(source) => {
                return Err(StateError::Unreadable {
                    path: path.to_path_buf(),
                    source: Box::new(source),
                })
            }
        };

        let stored: StoredState =
            serde_json::from_str(&text).map_err(|source| StateError::Unreadable {
                path: path.to_path_buf(),
                source: Box::new(source),
            })?;

        if stored.version != FORMAT_VERSION {
            return Err(StateError::UnknownVersion {
                path: path.to_path_buf(),
                found: stored.version,
                expected: FORMAT_VERSION,
            });
        }

        let mut by_number = BTreeMap::new();
        for (number, entry) in stored.pull_requests {
            by_number.insert(number, restore(&entry)?);
        }
        Ok(Self { by_number })
    }

    /// Writes the state, replacing any previous one whole.
    ///
    /// Written to a temporary file beside the target and renamed over it. A
    /// watcher writes this after every poll and can be killed at any moment;
    /// writing in place leaves a half-file that the next run reads as
    /// corruption, losing every remembered state. `rename` within one
    /// directory is atomic, so a reader sees the old file or the new one and
    /// never a partial one.
    ///
    /// # Errors
    ///
    /// [`StateError::Unwritable`] if the directory cannot be created, the
    /// temporary file cannot be written, or the rename fails.
    pub fn save(&self, path: &Path) -> Result<(), StateError> {
        let unwritable = |source: std::io::Error| StateError::Unwritable {
            path: path.to_path_buf(),
            source,
        };

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(unwritable)?;
        }

        let stored = StoredState {
            version: FORMAT_VERSION,
            pull_requests: self
                .by_number
                .iter()
                .map(|(number, watched)| (*number, store(watched)))
                .collect(),
        };
        let text =
            serde_json::to_string_pretty(&stored).map_err(|source| StateError::Unwritable {
                path: path.to_path_buf(),
                source: std::io::Error::other(source),
            })?;

        // Beside the target, not in a temp directory: `rename` is only atomic
        // within a filesystem, and the two can differ.
        let temporary = path.with_extension("json.writing");
        fs::write(&temporary, text).map_err(unwritable)?;
        fs::rename(&temporary, path).map_err(unwritable)
    }

    /// What was remembered about one pull request, if anything.
    #[must_use]
    pub fn remembered(&self, number: PrNumber) -> Option<&Watched> {
        self.by_number.get(&number.get())
    }

    /// Records what to compare against next time.
    pub fn remember(&mut self, number: PrNumber, watched: Watched) {
        self.by_number.insert(number.get(), watched);
    }

    /// Drops everything that is not in `open`.
    ///
    /// Called with the pull requests the latest poll actually saw. Without it
    /// the file grows for the life of the repository, and a number that
    /// reopens would be compared against a reason recorded before it closed —
    /// a transition that never happened.
    pub fn retain_only(&mut self, open: &[PrNumber]) {
        let still_open: std::collections::BTreeSet<u32> =
            open.iter().map(|number| number.get()).collect();
        self.by_number
            .retain(|number, _| still_open.contains(number));
    }

    /// How many pull requests are remembered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_number.len()
    }

    /// Whether nothing is remembered, which is what a first run looks like.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_number.is_empty()
    }
}

/// Flattens a remembered state into the file's shape.
fn store(watched: &Watched) -> StoredPullRequest {
    StoredPullRequest {
        reason: reason_name(watched.reason()).to_owned(),
        head: watched.head().as_str().to_owned(),
        review_request: match watched.review_request() {
            ReviewRequest::AskedOfYou => "asked-of-you",
            ReviewRequest::NotAskedOfYou => "not-asked-of-you",
        }
        .to_owned(),
        approval: match watched.approval() {
            ApprovalState::Approved => "approved",
            ApprovalState::NotApproved => "not-approved",
        }
        .to_owned(),
        required_checks: match watched.required_checks() {
            RequiredCheckHealth::SomeFailing => "some-failing",
            RequiredCheckHealth::NoneFailing => "none-failing",
        }
        .to_owned(),
        rereview: match watched.rereview() {
            Rereview::Due => "due",
            Rereview::NotDue => "not-due",
        }
        .to_owned(),
    }
}

/// Reads one back.
fn restore(entry: &StoredPullRequest) -> Result<Watched, StateError> {
    let reason = reason_of(&entry.reason).ok_or_else(|| StateError::UnknownReason {
        name: entry.reason.clone(),
    })?;
    let review_request = match entry.review_request.as_str() {
        "asked-of-you" => ReviewRequest::AskedOfYou,
        _ => ReviewRequest::NotAskedOfYou,
    };
    let approval = match entry.approval.as_str() {
        "approved" => ApprovalState::Approved,
        _ => ApprovalState::NotApproved,
    };
    let required_checks = match entry.required_checks.as_str() {
        "some-failing" => RequiredCheckHealth::SomeFailing,
        _ => RequiredCheckHealth::NoneFailing,
    };
    let rereview = match entry.rereview.as_str() {
        "due" => Rereview::Due,
        _ => Rereview::NotDue,
    };
    Ok(Watched::restored(
        reason,
        HeadSha::parse(&entry.head)?,
        review_request,
        approval,
        required_checks,
        rereview,
    ))
}

/// The name a rung is written under.
///
/// Spelled out rather than derived, because these strings are a file format:
/// renaming a variant must not silently invalidate every state file in
/// existence, and a `Debug` derive would do exactly that.
const fn reason_name(reason: ReasonKind) -> &'static str {
    match reason {
        ReasonKind::Draft => "draft",
        ReasonKind::ChangesRequested => "changes-requested",
        ReasonKind::RequiredCheckFailing => "required-check-failing",
        ReasonKind::UnresolvedThreads => "unresolved-threads",
        ReasonKind::ApprovedButConflicting => "approved-but-conflicting",
        ReasonKind::Conflicting => "conflicting",
        ReasonKind::BehindBase => "behind-base",
        ReasonKind::ApprovedCiRunning => "approved-ci-running",
        ReasonKind::ReadyToMerge => "ready-to-merge",
        ReasonKind::CiRunning => "ci-running",
        ReasonKind::MergeabilityNotKnown => "mergeability-not-known",
        ReasonKind::WaitingForReview => "waiting-for-review",
        ReasonKind::NobodyAssigned => "nobody-assigned",
    }
}

/// The rung a name means, or `None` for one this build does not know.
fn reason_of(name: &str) -> Option<ReasonKind> {
    [
        ReasonKind::Draft,
        ReasonKind::ChangesRequested,
        ReasonKind::RequiredCheckFailing,
        ReasonKind::UnresolvedThreads,
        ReasonKind::ApprovedButConflicting,
        ReasonKind::Conflicting,
        ReasonKind::BehindBase,
        ReasonKind::ApprovedCiRunning,
        ReasonKind::ReadyToMerge,
        ReasonKind::CiRunning,
        ReasonKind::MergeabilityNotKnown,
        ReasonKind::WaitingForReview,
        ReasonKind::NobodyAssigned,
    ]
    .into_iter()
    .find(|reason| reason_name(*reason) == name)
}

#[cfg(test)]
mod tests {
    use super::{StateError, WatchState};
    use prtrack_core::{
        ApprovalState, HeadSha, InvalidValue, PrNumber, ReasonKind, RequiredCheckHealth, Rereview,
        ReviewRequest, Watched,
    };
    use std::fs;
    use std::path::PathBuf;

    const A_HEAD: &str = "9f2c4e1b7a3d5068c9e4b2a17f36d8095c1e4a7b";
    const ANOTHER_HEAD: &str = "1a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d";

    /// A directory that cleans itself up, so a test never leaves a file behind
    /// and two tests never share a path.
    struct Scratch {
        path: PathBuf,
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("prtrack-state-{name}"));
            let _ignored = fs::remove_dir_all(&path);
            let created = fs::create_dir_all(&path);
            assert!(created.is_ok(), "could not make a scratch directory");
            Self { path }
        }

        fn file(&self) -> PathBuf {
            self.path.join("watch.json")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ignored = fs::remove_dir_all(&self.path);
        }
    }

    /// Errors here box a source and cannot derive `PartialEq`, so assertions
    /// compare what a reader would actually see.
    fn said(error: &StateError) -> String {
        error.to_string()
    }

    fn watched(head: &str, reason: ReasonKind) -> Result<Watched, InvalidValue> {
        Ok(Watched::restored(
            reason,
            HeadSha::parse(head)?,
            ReviewRequest::NotAskedOfYou,
            ApprovalState::Approved,
            RequiredCheckHealth::NoneFailing,
            Rereview::NotDue,
        ))
    }

    #[test]
    fn a_watcher_that_has_never_run_starts_from_nothing_rather_than_failing() {
        // The first run has no file, and that is the ordinary case rather than
        // an error. Treating it as one would make the watcher refuse to start
        // exactly once per machine.
        let scratch = Scratch::new("absent");
        let loaded = WatchState::load(&scratch.file())
            .map(|state| (state.len(), state.is_empty()))
            .map_err(|error| said(&error));
        assert_eq!(loaded, Ok((0, true)));
    }

    #[test]
    fn what_was_remembered_comes_back_the_same() {
        let scratch = Scratch::new("roundtrip");
        let survived = (|| -> Result<_, String> {
            let first = watched(A_HEAD, ReasonKind::ReadyToMerge).map_err(|e| e.to_string())?;
            let second = watched(ANOTHER_HEAD, ReasonKind::CiRunning).map_err(|e| e.to_string())?;
            let seven = PrNumber::new(7).map_err(|e| e.to_string())?;
            let eleven = PrNumber::new(11).map_err(|e| e.to_string())?;

            let mut state = WatchState::default();
            state.remember(seven, first.clone());
            state.remember(eleven, second);
            state.save(&scratch.file()).map_err(|e| said(&e))?;

            let reloaded = WatchState::load(&scratch.file()).map_err(|e| said(&e))?;
            Ok((
                reloaded.len(),
                reloaded.remembered(seven) == Some(&first),
                reloaded.remembered(eleven).map(Watched::reason),
            ))
        })();

        assert_eq!(survived, Ok((2, true, Some(ReasonKind::CiRunning))));
    }

    #[test]
    fn every_rung_survives_a_round_trip_under_its_own_name() {
        // The names are a file format. A `Debug` derive would have renamed
        // every one of them the next time a variant was renamed, silently
        // invalidating every state file in existence -- so they are spelled
        // out, and this is what proves the spelling round-trips.
        let scratch = Scratch::new("rungs");
        let every = [
            ReasonKind::Draft,
            ReasonKind::ChangesRequested,
            ReasonKind::RequiredCheckFailing,
            ReasonKind::UnresolvedThreads,
            ReasonKind::ApprovedButConflicting,
            ReasonKind::Conflicting,
            ReasonKind::BehindBase,
            ReasonKind::ApprovedCiRunning,
            ReasonKind::ReadyToMerge,
            ReasonKind::CiRunning,
            ReasonKind::MergeabilityNotKnown,
            ReasonKind::WaitingForReview,
            ReasonKind::NobodyAssigned,
        ];

        let restored = (|| -> Result<Vec<ReasonKind>, String> {
            let mut state = WatchState::default();
            for (index, reason) in every.iter().enumerate() {
                let number = u32::try_from(index + 1).map_err(|e| e.to_string())?;
                state.remember(
                    PrNumber::new(number).map_err(|e| e.to_string())?,
                    watched(A_HEAD, *reason).map_err(|e| e.to_string())?,
                );
            }
            state.save(&scratch.file()).map_err(|e| said(&e))?;
            let reloaded = WatchState::load(&scratch.file()).map_err(|e| said(&e))?;

            (1..=13_u32)
                .map(|number| {
                    let pr = PrNumber::new(number).map_err(|e| e.to_string())?;
                    reloaded
                        .remembered(pr)
                        .map(Watched::reason)
                        .ok_or_else(|| format!("#{number} was not restored"))
                })
                .collect()
        })();

        assert_eq!(restored, Ok(every.to_vec()));
    }

    #[test]
    fn a_pull_request_that_closed_is_forgotten() {
        // Without this the file grows for the life of the repository, and a
        // number that reopens would be compared against a reason recorded
        // before it closed -- a transition that never happened.
        let scratch = Scratch::new("prune");
        let kept = (|| -> Result<_, String> {
            let w = watched(A_HEAD, ReasonKind::ReadyToMerge).map_err(|e| e.to_string())?;
            let numbered = |n: u32| PrNumber::new(n).map_err(|e| e.to_string());
            let (first, closed, third) = (numbered(1)?, numbered(2)?, numbered(3)?);

            let mut state = WatchState::default();
            for number in [first, closed, third] {
                state.remember(number, w.clone());
            }
            state.retain_only(&[first, third]);
            state.save(&scratch.file()).map_err(|e| said(&e))?;

            let reloaded = WatchState::load(&scratch.file()).map_err(|e| said(&e))?;
            Ok((reloaded.len(), reloaded.remembered(closed).is_some()))
        })();

        assert_eq!(kept, Ok((2, false)));
    }

    #[test]
    fn the_file_is_replaced_whole_or_not_at_all() {
        // A watcher writes this after every poll and can be killed at any
        // moment. Writing in place leaves a half-file that the next run reads
        // as corruption, losing every remembered state and going silent for a
        // poll. The rename is what makes the swap atomic -- and the temporary
        // file must not survive it, or the next reader finds two files and has
        // to guess.
        let scratch = Scratch::new("atomic");
        let leftovers = (|| -> Result<Vec<String>, String> {
            let w = watched(A_HEAD, ReasonKind::ReadyToMerge).map_err(|e| e.to_string())?;

            let mut first = WatchState::default();
            first.remember(PrNumber::new(1).map_err(|e| e.to_string())?, w.clone());
            first.save(&scratch.file()).map_err(|e| said(&e))?;

            let mut second = WatchState::default();
            second.remember(PrNumber::new(2).map_err(|e| e.to_string())?, w);
            second.save(&scratch.file()).map_err(|e| said(&e))?;

            let mut names: Vec<String> = fs::read_dir(&scratch.path)
                .map_err(|e| e.to_string())?
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            Ok(names)
        })();

        assert_eq!(
            leftovers,
            Ok(vec!["watch.json".to_owned()]),
            "the temporary file must not survive the write that used it"
        );
    }

    #[test]
    fn a_state_file_that_cannot_be_read_says_so_rather_than_looking_empty() {
        // An unreadable file and a first run produce the same behaviour --
        // every pull request looks new, so nothing nudges -- and only one of
        // them is fine. The caller has to be able to say which happened.
        let scratch = Scratch::new("corrupt");
        let written = fs::write(scratch.file(), b"{ this is not json");
        assert!(written.is_ok(), "the fixture must be written");

        let loaded = WatchState::load(&scratch.file());
        assert!(
            matches!(loaded, Err(StateError::Unreadable { .. })),
            "expected an unreadable-file error, got {:?}",
            loaded.map(|state| state.len())
        );
    }

    #[test]
    fn a_file_from_a_future_version_is_refused_rather_than_guessed_at() {
        // Reading an unknown shape as absent would mean comparing against a
        // state that was never recorded, which shows up as a nudge that never
        // fires -- invisible, and the expensive direction.
        let scratch = Scratch::new("version");
        let written = fs::write(scratch.file(), br#"{"version":99,"pull_requests":{}}"#);
        assert!(written.is_ok());

        let loaded = WatchState::load(&scratch.file());
        assert!(
            matches!(loaded, Err(StateError::UnknownVersion { found: 99, .. })),
            "expected a version refusal, got {:?}",
            loaded.map(|state| state.len())
        );
    }

    #[test]
    fn a_rung_this_build_does_not_know_is_refused_by_name() {
        // A file written by a newer build that added a rung. Saying which name
        // it was is the difference between a bug report and a shrug.
        let scratch = Scratch::new("unknown-rung");
        let written = fs::write(
            scratch.file(),
            format!(
                r#"{{"version":1,"pull_requests":{{"1":{{"reason":"blocked-on-the-moon","head":"{A_HEAD}","review_request":"not-asked-of-you","approval":"approved","required_checks":"none-failing"}}}}}}"#
            ),
        );
        assert!(written.is_ok());

        let loaded = WatchState::load(&scratch.file());
        assert!(
            matches!(&loaded, Err(StateError::UnknownReason { name }) if name == "blocked-on-the-moon"),
            "expected the unknown rung to be named, got {:?}",
            loaded.map(|state| state.len())
        );
    }

    #[test]
    fn a_due_re_review_survives_a_restart() {
        // Losing it would make the next poll see "not due" and the one after
        // see "due" again -- the transition fired a second time, by a restart.
        let scratch = Scratch::new("rereview");
        let restored = (|| -> Result<_, String> {
            let due = Watched::restored(
                ReasonKind::ChangesRequested,
                HeadSha::parse(A_HEAD).map_err(|e| e.to_string())?,
                ReviewRequest::NotAskedOfYou,
                ApprovalState::NotApproved,
                RequiredCheckHealth::NoneFailing,
                Rereview::Due,
            );
            let one = PrNumber::new(101).map_err(|e| e.to_string())?;
            let mut state = WatchState::default();
            state.remember(one, due);
            state.save(&scratch.file()).map_err(|e| said(&e))?;

            let reloaded = WatchState::load(&scratch.file()).map_err(|e| said(&e))?;
            Ok(reloaded.remembered(one).map(Watched::rereview))
        })();

        assert_eq!(restored, Ok(Some(Rereview::Due)));
    }

    #[test]
    fn a_state_file_from_before_re_reviews_were_tracked_still_loads() {
        // Written by a build that did not know the field. Refusing it would
        // turn an upgrade into a first sighting for every pull request; reading
        // it as not due means the first push after the upgrade still nudges.
        let scratch = Scratch::new("pre-rereview");
        let written = fs::write(
            scratch.file(),
            format!(
                r#"{{"version":1,"pull_requests":{{"101":{{"reason":"changes-requested","head":"{A_HEAD}","review_request":"not-asked-of-you","approval":"not-approved","required_checks":"none-failing"}}}}}}"#
            ),
        );
        assert!(written.is_ok());

        let restored = WatchState::load(&scratch.file())
            .map_err(|error| said(&error))
            .and_then(|state| {
                let one = PrNumber::new(101).map_err(|e| e.to_string())?;
                Ok(state.remembered(one).map(Watched::rereview))
            });

        assert_eq!(restored, Ok(Some(Rereview::NotDue)));
    }
}
