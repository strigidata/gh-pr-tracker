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
use prtrack_cli::poll::poll;
use prtrack_cli::render::BoardText;
use prtrack_cli::screen::{self, Screen};
use prtrack_cli::state::WatchState;
use prtrack_cli::update;
use prtrack_cli::upgrade;
use prtrack_cli::watch::{already_waiting, tick, Nudged};
use prtrack_core::{Board, BoardRow, BranchName, Login, PolicyRules, RepoRef};
use prtrack_github::{GitHubClient, RepositoryFacts};

/// A failure, and the number it should leave behind.
///
/// The error chain and the exit code answer different audiences — a person
/// reading stderr and a script reading `$?` — so they travel together rather
/// than one being derived from the other at the point of printing.
struct Trouble {
    exit: Exit,
    cause: anyhow::Error,
    /// That a newer `prtrack` is out, where the run found out before failing.
    /// Printed under the error, because a failure is exactly when somebody on
    /// an old build most needs to know a newer one exists.
    notice: Option<String>,
}

impl Trouble {
    /// A failure that happened before anything could be asked about updates.
    const fn new(exit: Exit, cause: anyhow::Error) -> Self {
        Self {
            exit,
            cause,
            notice: None,
        }
    }

    /// A failure that a person must fix by changing what they typed.
    const fn usage(cause: anyhow::Error) -> Self {
        Self::new(Exit::Usage, cause)
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
            if let Some(notice) = trouble.notice {
                eprintln!("{notice}");
            }
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
        Command::Update(args) => update_binary(&args),
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
    if let Some(warning) = notify::prepare() {
        eprintln!("prtrack: {warning}");
    }

    if let notify::Focus::Active { mode } = notify::focus() {
        eprintln!(
            "prtrack: {mode} is on, so macOS will drop every notification without \
             showing it. Nudges will still be printed here."
        );
    }

    let path = state_path(args)?;
    let runtime = tokio::runtime::Runtime::new()
        .context("could not start the async runtime")
        .map_err(|cause| Trouble::new(Exit::Api, cause))?;

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
    let client = GitHubClient::from_env().map_err(|source| {
        Trouble::new(
            Exit::of(&source),
            anyhow::Error::new(source).context("could not prepare a GitHub client"),
        )
    })?;

    let mut state = remembered(path);
    let mut first_poll = true;
    // What a failing poll last said about a newer release. The footer does the
    // same job for a poll that worked, by replacing rather than accumulating.
    let mut announced = update::Announced::default();
    // Whether `Loading...` is still the only thing on screen. It is drawn
    // before the first fetch and has to be *replaced* rather than printed
    // over, by the board or by the failure — whichever arrives first.
    let mut showing_placeholder = false;

    // The board is repainted in place rather than printed again underneath, so
    // the terminal shows one board rather than a transcript of every poll. The
    // screen restores the cursor when this function returns by any route.
    let interactive = args.format == Format::Text && std::io::stdout().is_terminal();
    let mut screen = Screen::new(std::io::stdout(), interactive);

    // Anything worth saying goes here rather than to stderr while the screen is
    // being repainted, because a repaint erases from the cursor down and stderr
    // lines sit exactly there. Without this the nudges — the whole product —
    // survived until the next poll and no longer.
    let mut footer = screen::Footer::default();
    if interactive {
        // A Focus mode is a standing condition, not an event: it was announced
        // once at startup and erased by the first repaint, which is the worst
        // possible place to lose "your notifications are being swallowed".
        if let notify::Focus::Active { mode } = notify::focus() {
            footer.warnings.push(format!(
                "{mode} is on, so macOS will drop every notification without showing it."
            ));
        }
    }

    // Resolved for *stderr*, which is where a nudge goes. It is a different
    // stream from the board's and can be redirected on its own, so borrowing
    // stdout's answer would write escapes into a log the moment somebody ran
    // `--watch 2>nudges.log`.
    let stderr_palette = stream_palette(args, std::io::stderr().is_terminal());

    // Before the first request, not after it. The first fetch is the slow one
    // — a fresh connection and nothing cached to revalidate against — and an
    // empty terminal in the meantime is indistinguishable from a command that
    // never started.
    if interactive {
        screen.redraw(&screen::loading());
        showing_placeholder = true;
    }

    loop {
        match read_repository(&client, args).await {
            Ok((facts, update_notice)) => {
                let (viewer, default_branch, default_rule, pairs) = facts.into_parts();
                let judged: Vec<(BoardRow, PolicyRules)> = pairs
                    .into_iter()
                    .map(|(facts, policy)| (BoardRow::reduce(facts, &policy), policy))
                    .collect();
                let board =
                    Board::assemble(judged.iter().map(|(row, _)| row.clone()).collect(), &viewer);

                match default_branch
                    .as_ref()
                    .map(|branch| board_text(args, &board, branch, &default_rule))
                {
                    Some(rendered) => {
                        screen.redraw(&format!("{rendered}{}", footer.render(palette(args))));
                        showing_placeholder = false;
                    }
                    // JSON, or a repository with no default branch. Neither is
                    // a board to repaint, so fall back to writing it out.
                    None => write_board(
                        args,
                        &board,
                        &judged,
                        default_branch.as_ref(),
                        &default_rule,
                    ),
                }

                if first_poll {
                    announce_first_poll(&judged, &viewer);
                    first_poll = false;
                }

                for nudged in tick(&mut state, &judged, &viewer) {
                    let line = deliver(&nudged, stderr_palette, !interactive);
                    if interactive {
                        footer.saw(line);
                    }
                }

                // Once per poll, which the one-hour cache turns into once an
                // hour. A watcher left running for days is exactly who most
                // needs telling that it has been superseded.
                if let Some(notice) = update_notice {
                    if interactive {
                        // A condition rather than an event: it stays true until
                        // somebody upgrades, so it is replaced rather than
                        // accumulated.
                        footer.warnings.retain(|line| !line.starts_with('❗'));
                        footer.warnings.push(notice);
                    } else {
                        eprintln!("{notice}");
                    }
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
                // `Loading...` is still the only thing on screen, and the
                // cursor is sitting at the end of it. Erasing first puts the
                // failure where the board would have gone rather than on the
                // same line as the placeholder it replaces.
                if showing_placeholder {
                    screen.redraw("");
                    showing_placeholder = false;
                }
                eprintln!("prtrack: {:#}", trouble.cause);
                if !worth_retrying(trouble.exit) {
                    return Err(trouble);
                }
                // Printed rather than put in the footer: nothing repaints the
                // board while polls keep failing, so a footer line would never
                // be seen — and a run of failures is when it matters most.
                // Once, though: an outage repeats the same line every poll,
                // and a line said a hundred times an hour is one nobody reads.
                if let Some(notice) = &trouble.notice {
                    if announced.first_time(notice) {
                        eprintln!("{notice}");
                    }
                }
                eprintln!("prtrack: nothing recorded for this poll; trying again shortly.");
            }
        }

        if !wait_out_interval(&mut screen, interval).await {
            return Ok(Exit::Informational);
        }
    }
}

/// Sleeps out one interval, ticking the line under the board.
///
/// Broken into one-second steps rather than one long sleep so the line can be
/// rewritten as it counts, and so `Updating...` appears the moment the wait
/// ends rather than after the fetch that follows it.
///
/// Returns `false` when Ctrl-C arrived, which is the caller's cue to stop.
/// The signal is raced against every step: without that the process dies on it,
/// [`Screen`]'s destructor never runs, and the shell the user returns to has an
/// invisible cursor with no way to know why — the failure `AGENTS.md` means by
/// "a crash must restore the terminal".
async fn wait_out_interval(
    screen: &mut Screen<std::io::Stdout>,
    interval: std::time::Duration,
) -> bool {
    let seconds = i64::try_from(interval.as_secs()).unwrap_or(i64::MAX);
    for remaining in (1..=seconds).rev() {
        screen.status_line(&screen::countdown(remaining));
        tokio::select! {
            () = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
            result = tokio::signal::ctrl_c() => {
                // An unavailable handler is not a reason to keep running: the
                // user asked to stop either way.
                drop(result);
                return false;
            }
        }
    }
    screen.status_line(&screen::updating());
    true
}

/// One read of the repository and the version check that goes with it, on a
/// client the caller already owns.
///
/// The notice comes back either way: beside the facts when the read worked,
/// and inside the [`Trouble`] when it did not, so whoever prints the failure
/// prints the way out of it too.
async fn read_repository(
    client: &GitHubClient,
    args: &BoardArgs,
) -> Result<(RepositoryFacts, Option<String>), Trouble> {
    let check = update::UpdateCheck::for_this_machine();
    let polled = poll(client, &args.repo, check.as_ref(), Utc::now().timestamp()).await;
    let notice = polled.update.notice();

    let fetched = match polled.read {
        Ok(fetched) => fetched,
        Err(source) => {
            return Err(Trouble {
                exit: Exit::of(&source),
                cause: anyhow::Error::new(source)
                    .context(format!("could not read {} from GitHub", args.repo)),
                notice,
            })
        }
    };
    match RepositoryFacts::try_from(fetched) {
        Ok(facts) => Ok((facts, notice)),
        Err(source) => Err(Trouble {
            // A payload we cannot read is GitHub's answer being wrong, not the
            // caller's command line, so it is retryable in the same sense a
            // 500 is.
            exit: Exit::Api,
            cause: anyhow::Error::new(source).context("GitHub's answer could not be read"),
            notice,
        }),
    }
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
fn deliver(nudged: &Nudged, stderr_palette: Palette, to_stderr: bool) -> String {
    let summary = nudged.summary();

    // Where stderr is a terminal the number carries the link and the URL is not
    // printed: the line reads as a sentence and the pull request is one click
    // away. Where it is not — a redirected log, a pipe — the URL is spelled out,
    // because a link nothing can click is a URL that has been deleted.
    let line = {
        let linked = stderr_palette.link(nudged.url.as_str(), &nudged.label());
        if linked == nudged.label() {
            format!("{summary} — {}", nudged.url.as_str())
        } else {
            format!("{linked} {}", nudged.happened())
        }
    };
    // Printed here only when nothing is repainting the screen. A watcher draws
    // it into the frame instead, because a redraw erases from the cursor down
    // and would take this with it.
    if to_stderr {
        eprintln!("prtrack: {line}");
    }

    // The notification gets the plain form. Its body is not a terminal and
    // would show an escape sequence literally.
    if let Err(reason) = notify::post(&summary, nudged.title.as_str(), nudged.url.as_str()) {
        eprintln!("prtrack: that notification could not be posted: {reason:#}");
    } else if let notify::Focus::Active { mode } = notify::focus() {
        // Checked per nudge rather than once, because a Focus can begin
        // mid-session -- and the run where that happens is exactly the run
        // where somebody is about to conclude the tool is broken.
        if to_stderr {
            eprintln!("prtrack: ...but {mode} is on, so that banner was suppressed.");
        }
    }

    line
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
    let (facts, update_notice) = fetch(args)?;
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

    // Under the board, so it is the last thing read rather than the first. On
    // stderr, so `--format json | jq` never sees it.
    if let Some(notice) = update_notice {
        eprintln!("{notice}");
    }

    Ok(outcome(&board, &viewer))
}

/// Replaces this binary with the newest published release.
///
/// Prints to stderr, not stdout: this is the tool talking about itself, and
/// stdout is the board's.
fn update_binary(args: &cli::UpdateArgs) -> Result<Exit, Trouble> {
    let repo = match args.from.clone() {
        Some(chosen) => chosen,
        None => RepoRef::parse(&update::release_repo()).map_err(|source| {
            Trouble::usage(anyhow::Error::new(source).context("the release repository is unusable"))
        })?,
    };

    let client = GitHubClient::from_env().map_err(|source| {
        Trouble::new(
            Exit::of(&source),
            anyhow::Error::new(source).context("could not prepare a GitHub client"),
        )
    })?;

    let runtime = tokio::runtime::Runtime::new()
        .context("could not start the async runtime")
        .map_err(|cause| Trouble::new(Exit::Api, cause))?;

    let outcome = runtime
        .block_on(upgrade::run(&client, &repo))
        .map_err(|failure| {
            // Whatever went wrong, nothing was replaced: `upgrade::run` moves
            // the binary only after every check has passed. What differs is
            // whether trying again could help — an unwritable directory stays
            // unwritable, and telling a wrapper to retry it is a lie.
            let exit = if failure.retryable {
                Exit::Api
            } else {
                Exit::Usage
            };
            Trouble::new(exit, failure.cause)
        })?;

    eprintln!("prtrack: {}", outcome.describe());
    Ok(Exit::Informational)
}

/// Asks GitHub, and says which kind of failure it was if it will not answer.
fn fetch(args: &BoardArgs) -> Result<(RepositoryFacts, Option<String>), Trouble> {
    // The token is read from the environment and never from a flag. An
    // argument is visible to every process on the machine through `ps` and
    // survives in shell history; see `prtrack_cli::cli`.
    let client = GitHubClient::from_env().map_err(|source| {
        Trouble::new(
            Exit::of(&source),
            anyhow::Error::new(source).context("could not prepare a GitHub client"),
        )
    })?;

    let runtime = tokio::runtime::Runtime::new()
        .context("could not start the async runtime")
        .map_err(|cause| Trouble::new(Exit::Api, cause))?;

    // The notice is computed here, where the runtime and the client already
    // exist, but *not* printed here: it belongs under the board rather than
    // over it, and the board is written by the caller.
    runtime.block_on(read_repository(&client, args))
}

/// Writes the board to stdout in the shape that was asked for.
/// Renders the board as the text `--watch` repaints, or `None` where there is
/// no default branch to name.
///
/// Split from [`write_board`] because a watcher hands the text to
/// [`Screen::redraw`] rather than printing it: printing appends, and appending
/// is the thing `--watch` is not supposed to do.
fn board_text(
    args: &BoardArgs,
    board: &Board,
    default_branch: &BranchName,
    default_rule: &PolicyRules,
) -> String {
    BoardText {
        board,
        rules: default_rule,
        default_branch: default_branch.as_str(),
        now: Utc::now(),
        palette: palette(args),
        show_branch: !args.no_branch,
    }
    .render()
}

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
            match default_branch.map(|branch| board_text(args, board, branch, default_rule)) {
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
    stream_palette(args, std::io::stdout().is_terminal())
}

/// The same decision, for whichever stream the caller is about to write to.
///
/// `--color` and the environment apply to both; only the terminal test differs.
/// The board goes to stdout and a nudge goes to stderr, and either can be
/// redirected without the other, so each has to be asked about separately.
fn stream_palette(args: &BoardArgs, is_terminal: bool) -> Palette {
    let no_color = std::env::var("NO_COLOR").ok();
    let clicolor = std::env::var("CLICOLOR").ok();
    Palette::resolve(
        args.color,
        &Environment {
            no_color: no_color.as_deref(),
            clicolor: clicolor.as_deref(),
            stdout_is_terminal: is_terminal,
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
