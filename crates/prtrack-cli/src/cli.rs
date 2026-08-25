//! What the command line accepts.

use std::path::PathBuf;
use std::time::Duration;

use clap::{Args, Parser, Subcommand, ValueEnum};
use prtrack_core::RepoRef;

use crate::color::ColorChoice;

/// `prtrack` itself.
#[derive(Debug, Parser)]
#[command(
    name = "prtrack",
    version,
    about = "Every open pull request, and the reason it cannot merge yet."
)]
pub struct Cli {
    /// What to do.
    #[command(subcommand)]
    pub command: Command,
}

/// The things `prtrack` can be asked for.
///
/// A subcommand rather than a bare set of flags because `--watch` is coming,
/// and a program that grows a mode later reads better if it had one from the
/// start than if a flag quietly becomes a mode.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Show every open pull request and why it cannot merge yet.
    Board(BoardArgs),

    /// Replace this binary with the newest published release.
    ///
    /// Uses the same GitHub token the board does, so a private distribution
    /// needs nothing hosted — see `prtrack_cli::upgrade`.
    Update(UpdateArgs),
}

/// How an update was asked for.
#[derive(Debug, Args)]
pub struct UpdateArgs {
    /// Where releases are published, as `owner/name`.
    ///
    /// Defaults to where this binary came from. Overridable for a fork, or to
    /// test the path against somewhere else.
    #[arg(long, value_name = "OWNER/NAME", value_parser = repository)]
    pub from: Option<RepoRef>,
}

/// How the board was asked for.
#[derive(Debug, Args)]
pub struct BoardArgs {
    /// The repository to read, as `owner/name`.
    #[arg(long, value_name = "OWNER/NAME", value_parser = repository)]
    pub repo: RepoRef,

    /// `text` for a person, `json` for a machine.
    #[arg(long, default_value = "text")]
    pub format: Format,

    /// When to colour: `auto`, `always`, or `never`.
    ///
    /// `auto` paints only an interactive terminal, and only when neither
    /// `NO_COLOR` nor `CLICOLOR=0` objects.
    #[arg(long, default_value = "auto", value_parser = colour)]
    pub color: ColorChoice,

    /// Hide the branch column.
    ///
    /// The branch is shown by default because it is what you type to go and
    /// look at the work, and it is the one field the row otherwise makes you
    /// open GitHub for. Teams whose branch names are long can turn it off
    /// rather than let the row wrap.
    #[arg(long)]
    pub no_branch: bool,

    /// Keep watching, and notify when something changes.
    ///
    /// Refused when stdout is not a terminal: a watcher writing a board every
    /// few minutes into a pipe is somebody's disk filling up, and the
    /// notifications go to a desktop nobody is necessarily at.
    #[arg(long)]
    pub watch: bool,

    /// Seconds between polls while watching.
    ///
    /// Floored rather than trusted — see [`MINIMUM_INTERVAL_SECONDS`]. The
    /// default is five minutes, which is roughly how fast a review or a check
    /// changes anything a person would act on.
    #[arg(long, default_value_t = DEFAULT_INTERVAL_SECONDS, value_name = "SECONDS")]
    pub interval: u64,

    /// Where to keep what the watcher remembers between polls.
    ///
    /// Defaults to a per-repository file under the user's state directory.
    /// Overridable mainly so a run can be tried without disturbing a real
    /// watcher's memory.
    #[arg(long, value_name = "PATH")]
    pub state_file: Option<PathBuf>,
}

/// How long between polls unless told otherwise.
pub const DEFAULT_INTERVAL_SECONDS: u64 = 300;

/// The shortest interval this will actually use.
///
/// A poll costs a GraphQL request against a shared budget, and a watcher is
/// meant to run all day. Respecting the rate limit is not an optimisation
/// here; a watcher that exhausts it stops answering, which is the silence this
/// whole tool exists to prevent.
pub const MINIMUM_INTERVAL_SECONDS: u64 = 30;

impl BoardArgs {
    /// The interval to actually poll at.
    ///
    /// A value below the floor is raised rather than refused: somebody asking
    /// for `--interval 1` wants "as fast as reasonable", and failing the whole
    /// command over it helps nobody. The caller says on stderr that it was
    /// raised, so the number is not silently different from the one typed.
    #[must_use]
    pub const fn poll_interval(&self) -> Duration {
        let seconds = if self.interval < MINIMUM_INTERVAL_SECONDS {
            MINIMUM_INTERVAL_SECONDS
        } else {
            self.interval
        };
        Duration::from_secs(seconds)
    }
}

/// Which rendering to write to stdout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// Columns, for a person to read.
    Text,
    /// The board as data, for a script to read.
    Json,
}

/// Reads `--repo`.
///
/// # Errors
///
/// Returns the reason the reference could not be read, which `clap` renders as
/// a usage error. The message says what the argument should look like, because
/// the failure is nearly always a missing owner.
fn repository(raw: &str) -> Result<RepoRef, String> {
    RepoRef::parse(raw).map_err(|reason| format!("{reason} (expected OWNER/NAME)"))
}

/// Reads `--color`.
///
/// # Errors
///
/// Returns a message naming the three accepted values. An unrecognised one is
/// refused rather than quietly treated as `auto`: silently ignoring
/// `--color=yes` leaves the user with no way to learn they were not
/// understood.
fn colour(raw: &str) -> Result<ColorChoice, String> {
    ColorChoice::parse(raw).ok_or_else(|| format!("expected auto, always or never, got {raw:?}"))
}

/// Whether a `clap` refusal is really `--help` or `--version` succeeding.
///
/// `try_parse` returns `Err` for both, because neither produces a [`Cli`] — but
/// they are the command doing exactly what was asked of it. Treating every
/// refusal as a usage error exits `2` and prints a contradiction on stderr: the
/// command line was read perfectly, which is why there is help to show.
///
/// `use_stderr` is the predicate rather than a match on `ErrorKind` because it
/// is precisely the question being asked — did `clap` consider this worth
/// reporting as a failure — and it does not need a wildcard arm over an enum
/// that upstream may extend.
#[must_use]
pub fn is_informational(error: &clap::Error) -> bool {
    !error.use_stderr()
}

#[cfg(test)]
mod tests {
    use super::{is_informational, Cli, Command, Format};
    use crate::color::ColorChoice;
    use clap::Parser as _;

    /// Whether a command line was refused, and whether that refusal was really
    /// a success. `None` means it parsed.
    fn informational(args: &[&str]) -> Option<bool> {
        Cli::try_parse_from(args)
            .err()
            .map(|error| is_informational(&error))
    }

    #[test]
    fn asking_for_help_or_version_is_not_a_usage_error() {
        // Both come back as `Err` because neither yields a `Cli`, and both are
        // the command succeeding. A shell that treats exit 2 as "you typed it
        // wrong" would be told it did, by the one command that cannot be typed
        // wrong.
        assert_eq!(informational(&["prtrack", "--help"]), Some(true), "--help");
        assert_eq!(
            informational(&["prtrack", "--version"]),
            Some(true),
            "--version"
        );

        // The contrast is the point: a real mistake must stay a usage error, or
        // the fix has simply moved the lie.
        assert_eq!(
            informational(&["prtrack", "board", "--repo"]),
            Some(false),
            "a missing value is a genuine usage error"
        );
        assert_eq!(
            informational(&["prtrack", "board", "--repo", "octo-org/example"]),
            None,
            "a good command line is not refused at all"
        );
    }

    /// The board arguments a command line parsed to, or the reason it did not.
    fn board_of(args: &[&str]) -> Result<(String, Format, ColorChoice), String> {
        Cli::try_parse_from(args)
            .map(|cli| match cli.command {
                Command::Board(board) => (board.repo.to_string(), board.format, board.color),
                // Named rather than wildcarded, so a subcommand added later is
                // a compile error here rather than silently untested.
                Command::Update(_) => ("update".to_owned(), Format::Text, ColorChoice::Auto),
            })
            .map_err(|error| error.kind().to_string())
    }

    #[test]
    fn update_needs_no_arguments_and_takes_a_repository_if_offered() {
        // The whole point is that nobody has to remember anything, so the bare
        // word has to work.
        let parsed = |args: &[&str]| {
            Cli::try_parse_from(args).ok().map(|cli| match cli.command {
                Command::Update(update) => update
                    .from
                    .map_or_else(|| "default".to_owned(), |r| r.to_string()),
                Command::Board(_) => "board".to_owned(),
            })
        };

        assert_eq!(parsed(&["prtrack", "update"]), Some("default".to_owned()));
        assert_eq!(
            parsed(&["prtrack", "update", "--from", "octo-org/example"]),
            Some("octo-org/example".to_owned())
        );
    }

    #[test]
    fn a_board_is_asked_for_by_repository() {
        assert_eq!(
            board_of(&["prtrack", "board", "--repo", "octo-org/example"]),
            Ok((
                "octo-org/example".to_owned(),
                Format::Text,
                ColorChoice::Auto
            )),
            "text and auto are the defaults, so neither has to be typed"
        );
    }

    #[test]
    fn the_format_and_the_colour_are_read_from_the_command_line() {
        assert_eq!(
            board_of(&[
                "prtrack",
                "board",
                "--repo",
                "octo-org/example",
                "--format",
                "json",
                "--color",
                "never",
            ]),
            Ok((
                "octo-org/example".to_owned(),
                Format::Json,
                ColorChoice::Never
            ))
        );
    }

    #[test]
    fn a_repository_that_is_not_owner_slash_name_is_refused_at_the_command_line() {
        // Refused here rather than at the API, so the user is told what is
        // wrong with what they typed instead of what GitHub thought of it.
        for typed in ["example", "octo-org/", "/example", "a/b/c", ""] {
            assert!(
                board_of(&["prtrack", "board", "--repo", typed]).is_err(),
                "{typed:?} is not a repository reference"
            );
        }
    }

    #[test]
    fn a_repository_is_required_because_there_is_no_sensible_default() {
        // Guessing from the working directory's git remote would be a
        // different feature with its own failure modes, and a board silently
        // drawn for the wrong repository is worse than one not drawn at all.
        assert!(board_of(&["prtrack", "board"]).is_err());
    }

    #[test]
    fn an_unrecognised_colour_or_format_is_refused_rather_than_ignored() {
        for bad in [
            vec!["prtrack", "board", "--repo", "o/n", "--color", "yes"],
            vec!["prtrack", "board", "--repo", "o/n", "--format", "yaml"],
        ] {
            assert!(board_of(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn there_is_no_way_to_put_a_token_on_the_command_line() {
        // Deliberate, and worth a test rather than an absence somebody has to
        // notice. A token passed as an argument is readable by every other
        // process on the machine through `ps`, lands in shell history, and
        // gets pasted into bug reports along with the command that failed.
        //
        // `prtrack-github` reads GH_TOKEN or GITHUB_TOKEN itself, so there is
        // nothing for a flag to add: an override is what the variable already
        // is. The library cannot prevent a caller adding the flag, which makes
        // it this crate's job not to.
        for spelling in [
            vec!["prtrack", "board", "--repo", "o/n", "--token", "ghp_secret"],
            vec![
                "prtrack",
                "board",
                "--repo",
                "o/n",
                "--github-token",
                "ghp_secret",
            ],
            vec!["prtrack", "board", "--repo", "o/n", "-t", "ghp_secret"],
        ] {
            assert!(
                board_of(&spelling).is_err(),
                "{:?} must not be accepted",
                spelling.get(4)
            );
        }
    }

    #[test]
    fn the_repository_keeps_the_spelling_it_was_typed_in() {
        // `RepoRef` folds case for identity but renders what it was given, so
        // the board says the repository back the way the user asked for it.
        assert_eq!(
            board_of(&["prtrack", "board", "--repo", "Octo-Org/Example"]).map(|(repo, _, _)| repo),
            Ok("Octo-Org/Example".to_owned())
        );
    }
}
