//! Replacing this binary with the newest published one.
//!
//! Installing is four commands nobody remembers, and the version notice used to
//! print them at you as if you would. `prtrack update` is the same four steps
//! with the details filled in from where the binary actually lives.
//!
//! # It uses the token you already have
//!
//! Exactly as the version check does. A private repository's releases are
//! reachable by the people who can install from them, so there is nothing
//! hosted and no second credential — see [`crate::update`].
//!
//! # What it refuses to do
//!
//! **Replace a binary it cannot write.** Installed under `/usr/local/bin` on a
//! machine where that needs `sudo`, this says so and stops rather than
//! half-succeeding.
//!
//! **Install a download it cannot vouch for.** Every release publishes a
//! `.sha256` beside its tarball and the download is checked against it. HTTPS
//! already makes tampering unlikely; this catches the truncated download, which
//! is not unlikely at all and would otherwise install a binary that does not run.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _, Result};
use prtrack_github::{Asset, GitHubClient, Release};
use sha2::{Digest as _, Sha256};

use crate::update;

/// The asset name fragment identifying this platform's build.
///
/// macOS ships one universal binary rather than one per architecture, so there
/// is nothing to choose between on that side.
#[cfg(target_os = "macos")]
const PLATFORM: &str = "macos-universal";
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const PLATFORM: &str = "linux-x86_64";

/// What an update attempt concluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Already newest. Nothing was downloaded.
    Current {
        /// The version that is both running and published.
        version: String,
    },
    /// Replaced, and the new version is in place.
    Replaced {
        /// What was running before.
        from: String,
        /// What is running now.
        to: String,
        /// Where it was written.
        at: PathBuf,
    },
}

impl Outcome {
    /// The line to print.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Current { version } => {
                format!("prtrack {version} is the newest release; nothing to do.")
            }
            Self::Replaced { from, to, at } => {
                format!("prtrack {from} → {to}, installed at {}", at.display())
            }
        }
    }
}

/// Picks the asset for this platform out of a release.
///
/// Matched on the platform fragment rather than the whole filename, because the
/// filename carries the version and would have to be rebuilt here — which is
/// the same string being constructed in two places and drifting.
///
/// The `.sha256` sibling is excluded explicitly: it also contains the platform
/// fragment, and taking the first match would download the checksum and try to
/// run it.
#[must_use]
pub fn asset_for_platform(release: &Release) -> Option<&Asset> {
    release
        .assets
        .iter()
        .find(|asset| asset.name.contains(PLATFORM) && asset.name.ends_with(".tar.gz"))
}

/// The checksum published beside `asset`.
#[must_use]
pub fn checksum_for<'a>(release: &'a Release, asset: &Asset) -> Option<&'a Asset> {
    let wanted = format!("{}.sha256", asset.name);
    release.assets.iter().find(|other| other.name == wanted)
}

/// Whether `bytes` hashes to the digest named in a `shasum`-style line.
///
/// The published file is `<hex>  <filename>`, which is what both `shasum -a
/// 256` and `sha256sum` write, so only the first field is read.
#[must_use]
pub fn digest_matches(bytes: &[u8], published: &str) -> bool {
    let Some(expected) = published.split_whitespace().next() else {
        return false;
    };
    let actual = Sha256::digest(bytes);
    // Case-insensitive: the two tools agree on lowercase, but a checksum
    // rewritten by hand should not fail for its capitalisation.
    format!("{actual:x}").eq_ignore_ascii_case(expected)
}

/// Why an update did not happen, and whether trying again could help.
///
/// The exit code is a contract, and "the network hiccuped" and "you cannot
/// write to that directory" are different instructions to whoever is reading
/// it. Reporting both as retryable tells a wrapper to keep trying something
/// that will never work.
#[derive(Debug)]
pub struct Failure {
    /// True where another attempt might succeed — GitHub refused, the download
    /// broke. False where nothing changes until a person does something.
    pub retryable: bool,
    /// What went wrong, with its chain intact.
    pub cause: anyhow::Error,
}

impl Failure {
    /// Something that might work next time.
    const fn retryable(cause: anyhow::Error) -> Self {
        Self {
            retryable: true,
            cause,
        }
    }

    /// Something a person has to change.
    const fn settled(cause: anyhow::Error) -> Self {
        Self {
            retryable: false,
            cause,
        }
    }
}

/// Downloads the newest release and puts it where this binary is.
///
/// # Errors
///
/// Anything that stops a *complete and verified* replacement: GitHub refusing,
/// no build for this platform, a checksum that does not match, an unwritable
/// destination. Nothing is moved into place until every one of those has
/// passed.
pub async fn run(client: &GitHubClient, repo: &prtrack_core::RepoRef) -> Result<Outcome, Failure> {
    let running = update::running();

    let release = client
        .latest_release_detail(repo)
        .await
        .context("could not ask GitHub for the newest release")
        .map_err(Failure::retryable)?
        .with_context(|| format!("{repo} has published no releases"))
        .map_err(Failure::settled)?;

    let published = release.tag.strip_prefix('v').unwrap_or(&release.tag);
    if update::compare(running, Some(&release.tag)) == update::Update::Current {
        return Ok(Outcome::Current {
            version: running.to_owned(),
        });
    }

    let asset = asset_for_platform(&release)
        .with_context(|| {
            format!(
                "release {} has no {PLATFORM} build; the assets are: {}",
                release.tag,
                release
                    .assets
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .map_err(Failure::settled)?;

    let tarball = client
        .download_asset(&asset.url)
        .await
        .with_context(|| format!("could not download {}", asset.name))
        .map_err(Failure::retryable)?;

    // Verified before anything is unpacked, so a truncated download cannot
    // become a binary on the path.
    match checksum_for(&release, asset) {
        Some(sums) => {
            let published_sum = client
                .download_asset(&sums.url)
                .await
                .with_context(|| format!("could not download {}", sums.name))
                .map_err(Failure::retryable)?;
            let published_sum = String::from_utf8_lossy(&published_sum);
            if !digest_matches(&tarball, &published_sum) {
                // Retryable: the overwhelmingly likely cause is a download that
                // stopped early, and the next one may not.
                return Err(Failure::retryable(anyhow::anyhow!(
                    "{} does not match its published checksum; nothing was installed",
                    asset.name
                )));
            }
        }
        // Refused rather than shrugged at. A release with no checksum is a
        // release built by something this does not recognise, and installing
        // from it is the one irreversible step here.
        None => {
            return Err(Failure::settled(anyhow::anyhow!(
                "release {} publishes no checksum for {}; refusing to install unverified",
                release.tag,
                asset.name
            )))
        }
    }

    let destination = std::env::current_exe()
        .context("could not find where this binary lives")
        .map_err(Failure::settled)?;
    let installed = install(&tarball, &destination).map_err(Failure::settled)?;

    Ok(Outcome::Replaced {
        from: running.to_owned(),
        to: published.to_owned(),
        at: installed,
    })
}

/// Unpacks `tarball` and moves the binary over `destination`.
///
/// # Why a rename, and why into the same directory
///
/// A rename within one filesystem is atomic: either the old binary is there or
/// the new one is, never half of either. Unpacking to `/tmp` and moving would
/// cross a filesystem on most machines, which turns the move into a copy and
/// puts a partially written binary on the path if it fails half way.
///
/// Replacing a *running* executable is fine on macOS and Linux — the running
/// process keeps the old inode, and the next invocation gets the new one.
fn install(tarball: &[u8], destination: &Path) -> Result<PathBuf> {
    let directory = destination
        .parent()
        .context("the binary has no parent directory to write into")?;

    writable(directory, destination)?;

    let staging = directory.join(".prtrack-update");
    drop(std::fs::remove_dir_all(&staging));
    std::fs::create_dir_all(&staging)
        .with_context(|| format!("could not prepare {}", staging.display()))?;

    let archive = staging.join("prtrack.tar.gz");
    std::fs::write(&archive, tarball)
        .with_context(|| format!("could not write {}", archive.display()))?;

    // `tar` rather than a crate: both platforms ship it, and it is the same
    // command the install instructions tell a person to run by hand.
    let unpacked = std::process::Command::new("tar")
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(&staging)
        .status()
        .context("could not run tar")?;
    if !unpacked.success() {
        drop(std::fs::remove_dir_all(&staging));
        bail!("tar could not unpack the release");
    }

    let fresh = staging.join("prtrack");
    if !fresh.is_file() {
        drop(std::fs::remove_dir_all(&staging));
        bail!("the release did not contain a `prtrack` binary");
    }
    executable(&fresh)?;

    std::fs::rename(&fresh, destination).with_context(|| {
        format!(
            "could not replace {}; the new binary is at {}",
            destination.display(),
            fresh.display()
        )
    })?;
    drop(std::fs::remove_dir_all(&staging));

    Ok(destination.to_path_buf())
}

/// Refuses early where the replacement would fail late.
///
/// Checked before anything is downloaded, so `prtrack update` under
/// `/usr/local/bin` says what is wrong in a second rather than after a
/// five-megabyte download.
fn writable(directory: &Path, destination: &Path) -> Result<()> {
    let probe = directory.join(".prtrack-write-probe");
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            drop(std::fs::remove_file(&probe));
            Ok(())
        }
        Err(source) => Err(anyhow::Error::new(source).context(format!(
            "cannot write to {}, so {} cannot be replaced. Re-run with the \
             permission to write there, or install the release by hand",
            directory.display(),
            destination.display()
        ))),
    }
}

/// Marks the unpacked binary executable.
///
/// `tar` preserves the mode from the archive, and the archive is built by CI
/// from a file that already had it — but a release repackaged by anything else
/// would arrive without, and a downloaded binary that will not run is a
/// confusing way to find that out.
#[cfg(unix)]
fn executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .with_context(|| format!("could not make {} executable", path.display()))
}

#[cfg(not(unix))]
fn executable(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{asset_for_platform, checksum_for, digest_matches, Outcome, PLATFORM};
    use prtrack_github::{Asset, Release};
    use sha2::{Digest as _, Sha256};
    use std::path::PathBuf;

    fn release(names: &[&str]) -> Release {
        Release {
            tag: "v9.9.9".to_owned(),
            assets: names
                .iter()
                .map(|name| Asset {
                    name: (*name).to_owned(),
                    url: format!("https://api.example.invalid/{name}"),
                })
                .collect(),
        }
    }

    #[test]
    fn the_checksum_is_not_mistaken_for_the_download() {
        // Both names contain the platform fragment, so taking the first match
        // downloads the `.sha256`, installs it, and leaves a binary that is
        // sixty bytes of hex.
        let published = release(&[
            &format!("prtrack-v9.9.9-{PLATFORM}.tar.gz.sha256"),
            &format!("prtrack-v9.9.9-{PLATFORM}.tar.gz"),
        ]);

        assert_eq!(
            asset_for_platform(&published).map(|asset| asset.name.as_str()),
            Some(format!("prtrack-v9.9.9-{PLATFORM}.tar.gz").as_str()),
            "the tarball, not the checksum beside it"
        );
    }

    #[test]
    fn a_release_without_this_platform_is_not_installed_from() {
        // Better to say "there is no build for you" than to install another
        // platform's binary, which fails in a way nobody can read.
        let published = release(&["prtrack-v9.9.9-freebsd-riscv.tar.gz"]);
        assert_eq!(asset_for_platform(&published), None);
    }

    #[test]
    fn a_checksum_is_matched_to_its_own_tarball() {
        let published = release(&[
            &format!("prtrack-v9.9.9-{PLATFORM}.tar.gz"),
            &format!("prtrack-v9.9.9-{PLATFORM}.tar.gz.sha256"),
            "prtrack-v9.9.9-other-platform.tar.gz.sha256",
        ]);
        assert_eq!(
            asset_for_platform(&published)
                .and_then(|asset| checksum_for(&published, asset))
                .map(|sums| sums.name.as_str()),
            Some(format!("prtrack-v9.9.9-{PLATFORM}.tar.gz.sha256").as_str()),
            "the sibling, not whichever checksum came first"
        );
    }

    #[test]
    fn a_download_that_does_not_hash_to_its_checksum_is_refused() {
        // The realistic failure is a truncated download, not tampering. It
        // installs perfectly and does not run.
        let bytes = b"the release, entire";
        let correct = format!("{:x}  prtrack.tar.gz", Sha256::digest(bytes));

        assert!(digest_matches(bytes, &correct));
        assert!(
            digest_matches(bytes, &correct.to_uppercase()),
            "capitalisation is not a mismatch"
        );
        assert!(
            !digest_matches(b"the release, truncat", &correct),
            "a short download must not pass"
        );
        assert!(
            !digest_matches(bytes, ""),
            "an empty checksum is not a pass"
        );
    }

    #[test]
    fn being_current_reads_as_nothing_to_do_rather_than_as_success() {
        // Somebody who runs this twice should be told the second run did
        // nothing, not congratulated for installing what was already there.
        let current = Outcome::Current {
            version: "1.2.3".to_owned(),
        }
        .describe();
        assert!(current.contains("nothing to do"), "{current}");

        let replaced = Outcome::Replaced {
            from: "1.2.3".to_owned(),
            to: "1.2.4".to_owned(),
            at: PathBuf::from("/somewhere/prtrack"),
        }
        .describe();
        assert!(replaced.contains("1.2.3 → 1.2.4"), "{replaced}");
        assert!(replaced.contains("/somewhere/prtrack"), "and says where");
    }
}
