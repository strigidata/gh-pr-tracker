//! The `prtrack` command line.
//!
//! Everything here is plumbing. The decisions were made in `prtrack-core`, the
//! fetching in `prtrack-github`, and the drawing in this crate's library half;
//! this connects them and settles the three things only a process can answer —
//! what the clock says, where the output is going, and what to tell the shell
//! when it is over.
//!
//! # stdout is the data channel
//!
//! The board goes to stdout and nothing else does. Every diagnostic, including
//! the error chain, goes to stderr, so `prtrack board --format json | jq` works
//! whether or not the run went well.

use std::io::IsTerminal as _;
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use chrono::Utc;
use clap::Parser as _;
use prtrack_cli::cli::{self, BoardArgs, Cli, Command, Format, MINIMUM_INTERVAL_SECONDS};
use prtrack_cli::color::{Environment, Palette};
use prtrack_cli::exit::Exit;
use prtrack_cli::json::BoardJson;
use prtrack_cli::notify;
use prtrack_cli::render::BoardText;
use prtrack_cli::state::WatchState;
use prtrack_cli::watch::{already_waiting, tick, Nudged};
use prtrack_core::{Board, BoardRow, BranchName, Login, PolicyRules};
use prtrack_github::{GitHubClient, RepositoryFacts};

/// A failure, and the number it should leave behind.
///
/// The error chain and the exit code answer different audiences — a person
/// reading stderr and a script reading `$?` — so they travel together rather
/// than one being derived from the other at the point of printing.
struct Trouble {
    exit: Exit,
    cause: anyhow::Error,
}

impl Trouble {
    /// A failure that a person must fix by changing what they typed.
    const fn usage(cause: anyhow::Error) -> Self {
        Self {
            exit: Exit::Usage,
            cause,
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(exit) => exit.into(),
        Err(trouble) => {
            // The whole chain, because the useful sentence is usually not the
            // outermost one: "could not read the board" says nothing that
            // "GitHub refused the token" does not.
            eprintln!("prtrack: {:#}", trouble.cause);
            trouble.exit.into()
        }
    }
}

/// Does the work, leaving `main` to do nothing but convert the answer.
fn run() -> Result<Exit, Trouble> {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            // `clap` has already written its own message, which is better than
            // anything reconstructed here — and it sends help to stdout and a
            // complaint to stderr, so the stream split is already right.
            let _rendered = error.print();

            // `--help` and `--version` arrive here too, because neither yields
            // a `Cli`. They are the command succeeding, and exit 0 rather than
            // the usage code.
            return if cli::is_informational(&error) {
                Ok(Exit::Informational)
            } else {
                Err(Trouble::usage(anyhow::anyhow!(
                    "the command line could not be read"
                )))
            };
        }
    };

    match cli.command {
        Command::Board(args) if args.watch => watch(&args),
        Command::Board(args) => board(&args),
    }
}

/// Polls until interrupted, notifying when something changes.
///
/// Refused when stdout is not a terminal, falling back to one render: a
/// watcher writing a board every few minutes into a pipe is somebody's disk
/// filling up, and its notifications go to a desktop nobody is necessarily at.
fn watch(args: &BoardArgs) -> Result<Exit, Trouble> {
    if !std::io::stdout().is_terminal() {
        eprintln!(
            "prtrack: --watch needs a terminal, and stdout is not one. \
             Rendering the board once instead."
        );
        return board(args);
    }

    let interval = args.poll_interval();
    if args.interval < MINIMUM_INTERVAL_SECONDS {
        eprintln!(
            "prtrack: polling every {}s rather than the {}s asked for, to leave the \
             rate limit something to work with.",
            interval.as_secs(),
            args.interval
        );
    }

    // Once, before the first notification. macOS attributes a notification to
    // an application bundle and silently drops one it cannot attribute; see
    // `prtrack_cli::notify`.
    if let Err(reason) = notify::adopt_bundle_identity() {
        eprintln!("prtrack: notifications may not appear: {reason:#}");
    }

    let path = state_path(args)?;
    let runtime = tokio::runtime::Runtime::new()
        .context("could not start the async runtime")
        .map_err(|cause| Trouble {
            exit: Exit::Api,
            cause,
        })?;

    // The whole loop runs inside one `block_on`, and the client is built once
    // outside it. Both matter. Entering and leaving the runtime around every
    // poll drops a `reqwest::Client` — and its connection pool, whose tasks
    // belong to the runtime — from outside the async context, which wedges the
    // next timer: measured here as a `sleep(30s)` that had not returned after
    // ninety. Keeping one client also keeps the connection alive between
    // polls, which is the difference between a handshake every five minutes
    // and one a day.
    runtime.block_on(watch_forever(args, interval, &path))
}

/// The poll loop proper. Never returns while the failures are ones another
/// poll might get past.
async fn watch_forever(
    args: &BoardArgs,
    interval: std::time::Duration,
    path: &std::path::Path,
) -> Result<Exit, Trouble> {
    let client = GitHubClient::from_env().map_err(|source| Trouble {
        exit: Exit::of(&source),
        cause: anyhow::Error::new(source).context("could not prepare a GitHub client"),
    })?;

    let mut state = remembered(path);
    let mut first_poll = true;

    loop {
        match read_repository(&client, args).await {
            Ok(facts) => {
                let (viewer, default_branch, default_rule, pairs) = facts.into_parts();
                let judged: Vec<(BoardRow, PolicyRules)> = pairs
                    .into_iter()
                    .map(|(facts, policy)| (BoardRow::reduce(facts, &policy), policy))
                    .collect();
                let board =
                    Board::assemble(judged.iter().map(|(row, _)| row.clone()).collect(), &viewer);
                write_board(
                    args,
                    &board,
                    &judged,
                    default_branch.as_ref(),
                    &default_rule,
                );

                if first_poll {
                    announce_first_poll(&judged, &viewer);
                    first_poll = false;
                }

                for nudged in tick(&mut state, &judged, &viewer) {
                    deliver(&nudged);
                }
                if let Err(reason) = state.save(path) {
                    eprintln!(
                        "prtrack: could not save what was seen ({reason}); the next poll \
                         will treat everything as new and stay quiet."
                    );
                }
            }
            Err(trouble) => {
                // Not an observation. The state is left exactly as it was, so
                // the next successful poll compares against the last thing we
                // actually saw rather than across a gap — diffing across one
                // hides the transitions that happened inside it.
                eprintln!("prtrack: {:#}", trouble.cause);
                if !worth_retrying(trouble.exit) {
                    return Err(trouble);
                }
                eprintln!("prtrack: nothing recorded for this poll; trying again shortly.");
            }
        }

        tokio::time::sleep(interval).await;
    }
}

/// One read of the repository, on a client the loop already owns.
async fn read_repository(
    client: &GitHubClient,
    args: &BoardArgs,
) -> Result<RepositoryFacts, Trouble> {
    let fetched = client
        .fetch_open_pull_requests(&args.repo)
        .await
        .map_err(|source| Trouble {
            exit: Exit::of(&source),
            cause: anyhow::Error::new(source)
                .context(format!("could not read {} from GitHub", args.repo)),
        })?;
    RepositoryFacts::try_from(fetched).map_err(|source| Trouble {
        exit: Exit::Api,
        cause: anyhow::Error::new(source).context("GitHub's answer could not be read"),
    })
}

/// Whether a failure is one another poll might get past.
///
/// A refused credential or a repository we cannot see will still be refused in
/// five minutes, and a watcher that keeps asking burns the rate limit it needs
/// for the moment somebody fixes it.
const fn worth_retrying(exit: Exit) -> bool {
    matches!(exit, Exit::Api)
}

/// Says what the board already holds, and that notifications start now.
///
/// Both halves matter. Without the first, a first run implies there is nothing
/// to do — but every state it counts is a *condition*, so none of them will
/// ever produce a nudge, and a pull request approved-and-conflicted for three
/// weeks would sit there unmentioned. Without the second, the silence that
/// follows reads as breakage.
fn announce_first_poll(judged: &[(BoardRow, PolicyRules)], viewer: &Login) {
    let waiting = already_waiting(judged, viewer);
    match waiting.describe() {
        Some(already) => eprintln!(
            "prtrack: watching {} pull request(s); already on the board: {already}. \
             Notifications start from the next change.",
            judged.len()
        ),
        None => eprintln!(
            "prtrack: watching {} pull request(s); nothing on the board needs you yet. \
             Notifications start from the next change.",
            judged.len()
        ),
    }
}

/// Puts one nudge on screen, and says on stderr that it was attempted.
///
/// Attempted, not delivered: macOS drops a notification silently when a Focus
/// mode is filtering or notifications are off for the owning application, and
/// there is no way for this process to tell. The stderr line is the only
/// record a user has that something was meant to reach them, which is why it
/// is printed whether or not the banner appears.
fn deliver(nudged: &Nudged) {
    let summary = nudged.summary();
    eprintln!("prtrack: {summary} — {}", nudged.url.as_str());
    if let Err(reason) = notify::post(&summary, nudged.title.as_str()) {
        eprintln!("prtrack: that notification could not be posted: {reason:#}");
    }
}

/// Reads what a previous run remembered, saying so if it cannot.
///
/// A corrupt file is reported and then ignored rather than fatal. Starting
/// fresh costs one quiet poll; refusing to start costs the whole watch, and
/// the reader now knows which happened.
fn remembered(path: &std::path::Path) -> WatchState {
    match WatchState::load(path) {
        Ok(state) => state,
        Err(reason) => {
            eprintln!("prtrack: {reason}; starting from nothing, so this poll will be quiet.");
            WatchState::default()
        }
    }
}

/// Where this repository's watch state lives.
fn state_path(args: &BoardArgs) -> Result<std::path::PathBuf, Trouble> {
    if let Some(chosen) = args.state_file.clone() {
        return Ok(chosen);
    }
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|home| std::path::PathBuf::from(home).join(".local").join("state"))
        })
        .ok_or_else(|| {
            Trouble::usage(anyhow::anyhow!(
                "no HOME or XDG_STATE_HOME to keep watch state in; pass --state-file"
            ))
        })?;
    // One file per repository: two watchers on different repositories must not
    // read each other's memory and conclude everything changed.
    Ok(base.join("prtrack").join(format!(
        "watch-{}-{}.json",
        args.repo.owner(),
        args.repo.name()
    )))
}

/// Fetches a repository's open pull requests and writes the board.
fn board(args: &BoardArgs) -> Result<Exit, Trouble> {
    let facts = fetch(args)?;
    let (viewer, default_branch, default_rule, pairs) = facts.into_parts();

    // Each row keeps the rule that judged it: the JSON reports `governed` and
    // reproduces the oracle's sort keys, and neither can be recovered from a
    // row once it is classified.
    let judged: Vec<(BoardRow, PolicyRules)> = pairs
        .into_iter()
        .map(|(facts, policy)| (BoardRow::reduce(facts, &policy), policy))
        .collect();
    let board = Board::assemble(judged.iter().map(|(row, _)| row.clone()).collect(), &viewer);

    write_board(
        args,
        &board,
        &judged,
        default_branch.as_ref(),
        &default_rule,
    );
    Ok(outcome(&board, &viewer))
}

/// Asks GitHub, and says which kind of failure it was if it will not answer.
fn fetch(args: &BoardArgs) -> Result<RepositoryFacts, Trouble> {
    // The token is read from the environment and never from a flag. An
    // argument is visible to every process on the machine through `ps` and
    // survives in shell history; see `prtrack_cli::cli`.
    let client = GitHubClient::from_env().map_err(|source| Trouble {
        exit: Exit::of(&source),
        cause: anyhow::Error::new(source).context("could not prepare a GitHub client"),
    })?;

    let runtime = tokio::runtime::Runtime::new()
        .context("could not start the async runtime")
        .map_err(|cause| Trouble {
            exit: Exit::Api,
            cause,
        })?;

    let fetched = runtime
        .block_on(client.fetch_open_pull_requests(&args.repo))
        .map_err(|source| Trouble {
            exit: Exit::of(&source),
            cause: anyhow::Error::new(source)
                .context(format!("could not read {} from GitHub", args.repo)),
        })?;

    RepositoryFacts::try_from(fetched).map_err(|source| Trouble {
        // A payload we cannot read is GitHub's answer being wrong, not the
        // caller's command line, so it is retryable in the same sense a 500 is.
        exit: Exit::Api,
        cause: anyhow::Error::new(source).context("GitHub's answer could not be read"),
    })
}

/// Writes the board to stdout in the shape that was asked for.
fn write_board(
    args: &BoardArgs,
    board: &Board,
    judged: &[(BoardRow, PolicyRules)],
    default_branch: Option<&BranchName>,
    default_rule: &PolicyRules,
) {
    match args.format {
        Format::Text => {
            // No default branch means a repository with no commits, and
            // therefore no pull requests: the board below is empty and the
            // policy line would be a sentence about nothing. It is omitted
            // rather than given a placeholder, because a line naming a branch
            // is a line a reader will believe.
            let text = default_branch.map(|branch| {
                BoardText {
                    board,
                    rules: default_rule,
                    default_branch: branch.as_str(),
                    now: Utc::now(),
                    palette: palette(args),
                }
                .render()
            });
            match text {
                Some(rendered) => print!("{rendered}"),
                None => eprintln!(
                    "prtrack: this repository has no default branch, so there is no policy \
                     to report and nothing open to report it against."
                ),
            }
        }
        Format::Json => {
            let document = BoardJson {
                board,
                judged,
                // Null rather than a placeholder: JSON has a way to say
                // "absent" and the oracle uses it, so the two agree.
                default_branch: default_branch.map(BranchName::as_str),
                default_rule,
            }
            .to_value();
            println!("{document:#}");
        }
    }
}

/// Settles whether to paint, from the flag and from where stdout is going.
///
/// The only place in this program that reads the environment for colour. Below
/// here it is a decision already made, so nothing can paint against a
/// `NO_COLOR` it did not know about.
fn palette(args: &BoardArgs) -> Palette {
    let no_color = std::env::var("NO_COLOR").ok();
    let clicolor = std::env::var("CLICOLOR").ok();
    Palette::resolve(
        args.color,
        &Environment {
            no_color: no_color.as_deref(),
            clicolor: clicolor.as_deref(),
            stdout_is_terminal: std::io::stdout().is_terminal(),
        },
    )
}

/// What to tell the shell, and what to tell the person if it is nothing.
///
/// An empty board and a board that could not be fetched print almost the same
/// thing, so the difference is carried by the exit code — and said on stderr
/// too, because a person running this by hand does not check `$?`.
fn outcome(board: &Board, viewer: &Login) -> Exit {
    if board.total() == 0 {
        eprintln!(
            "prtrack: no open pull requests. This is an empty repository, not a \
             failed read — a failed read exits {}.",
            Exit::Api.code()
        );
        return Exit::NothingOpen;
    }
    if board.yours().is_empty() && board.needs_your_review().is_empty() {
        eprintln!(
            "prtrack: nothing here is waiting on {}, but {} pull request(s) are open.",
            viewer.as_str(),
            board.total()
        );
    }
    Exit::Board
}
