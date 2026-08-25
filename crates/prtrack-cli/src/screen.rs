//! Redrawing a board in place, and putting the terminal back afterwards.
//!
//! `--watch` replaces what is on screen rather than printing another copy
//! underneath it. Two things make that harder than moving the cursor home.
//!
//! **A shorter board must not leave the tail of a longer one behind.** Five
//! pull requests merging between polls would otherwise leave five stale rows
//! below the new board, indistinguishable from real ones. Every redraw erases
//! from the cursor to the end of the screen.
//!
//! **The terminal is borrowed, not owned.** The cursor is hidden while
//! redrawing, because a caret parked mid-board flickers on every repaint — and
//! anything hidden has to be restored, on a clean exit, on an error, and on
//! Ctrl-C. [`Screen`] restores on `Drop`, which covers the first two; the caller
//! must catch the signal for the third, because a process killed by `SIGINT`
//! never runs a destructor.
//!
//! The sink is a parameter rather than `stdout` so a test can read back exactly
//! what would have been written. Escape sequences are the entire content of
//! this module, and a test that cannot see them is testing nothing.

use std::io::Write;

/// Move the cursor to the top left, without clearing anything.
const HOME: &str = "\u{1b}[H";

/// Erase from the cursor to the end of the screen.
const CLEAR_BELOW: &str = "\u{1b}[J";

/// Erase from the cursor to the end of the line.
const CLEAR_LINE: &str = "\u{1b}[K";

/// Hide and show the caret.
const HIDE_CURSOR: &str = "\u{1b}[?25l";
const SHOW_CURSOR: &str = "\u{1b}[?25h";

/// A terminal being redrawn in place, which restores itself when dropped.
///
/// Holding a value rather than calling free functions is the point: the cursor
/// is hidden for as long as this exists and shown again when it does not, so
/// there is no path out of the watch loop that forgets to restore it.
#[derive(Debug)]
pub struct Screen<W: Write> {
    out: W,
    /// Whether we are driving a terminal. When false the control sequences are
    /// skipped, so a `--watch` that fell back to piped output writes none.
    interactive: bool,
}

impl<W: Write> Screen<W> {
    /// Takes over the screen, hiding the cursor.
    ///
    /// `interactive` should be false whenever the output is not a terminal.
    /// The sequences are then skipped rather than written into a file.
    pub fn new(out: W, interactive: bool) -> Self {
        let mut screen = Self { out, interactive };
        if interactive {
            screen.emit(HIDE_CURSOR);
        }
        screen
    }

    /// Replaces everything on screen with `body`.
    ///
    /// Home, then the new content, then erase whatever the old content left
    /// below it. Erasing *after* writing rather than clearing first is what
    /// stops the board flickering: the cells are overwritten in place and only
    /// the surplus is blanked.
    pub fn redraw(&mut self, body: &str) {
        if self.interactive {
            self.emit(HOME);
            self.emit(body);
            self.emit(CLEAR_BELOW);
        } else {
            // Not a terminal: no cursor to move and nothing to erase, so print
            // the board and let it scroll.
            self.emit(body);
        }
    }

    /// Rewrites the line under the board, without touching the board itself.
    ///
    /// Used once a second for the countdown. A carriage return and an erase are
    /// enough because the line is the only thing below the board, and leaving
    /// the cursor on it is what makes the next call cheap.
    pub fn status_line(&mut self, text: &str) {
        if !self.interactive {
            return;
        }
        self.emit("\r");
        self.emit(CLEAR_LINE);
        self.emit(text);
    }

    /// Writes and flushes. A dropped write is a half-drawn board.
    fn emit(&mut self, text: &str) {
        drop(self.out.write_all(text.as_bytes()));
        drop(self.out.flush());
    }
}

impl<W: Write> Drop for Screen<W> {
    /// Gives the cursor back.
    ///
    /// A watcher that exits leaving the caret invisible has broken the shell
    /// the user returns to, and the shell has no way to know it happened.
    fn drop(&mut self) {
        if self.interactive {
            // A newline first: the countdown left the cursor mid-line, and a
            // prompt printed on top of it reads as corruption.
            self.emit("\n");
            self.emit(SHOW_CURSOR);
        }
    }
}

/// The line under the board, counting down to the next poll.
///
/// Seconds rather than a bar, because the useful question is "have I got time
/// to read this" and a number answers it exactly. Zero and below read as
/// updating: a poll that overruns its interval must not show a negative.
#[must_use]
pub fn countdown(seconds_left: i64) -> String {
    match seconds_left {
        1 => "Next update in 1 second".to_owned(),
        n if n > 1 => format!("Next update in {n} seconds"),
        _ => updating(),
    }
}

/// The same line while a fetch is in flight.
#[must_use]
pub fn updating() -> String {
    "Updating...".to_owned()
}

#[cfg(test)]
mod tests {
    use super::{countdown, updating, Screen, CLEAR_BELOW, CLEAR_LINE, HIDE_CURSOR, SHOW_CURSOR};

    /// What a `Screen` wrote, once it has been dropped.
    fn written(interactive: bool, body: &str, status: &str) -> String {
        let mut sink: Vec<u8> = Vec::new();
        {
            let mut screen = Screen::new(&mut sink, interactive);
            screen.redraw(body);
            screen.status_line(status);
        }
        String::from_utf8_lossy(&sink).into_owned()
    }

    #[test]
    fn a_redraw_erases_whatever_the_last_one_left_below_it() {
        let out = written(true, "board", "Next update in 5 seconds");

        // The erase must come *after* the body. Clearing first would blank the
        // screen on every poll, which is a visible flicker; clearing after
        // overwrites in place and blanks only the surplus.
        assert_eq!(
            (
                out.contains("board"),
                out.find(CLEAR_BELOW) > out.find("board")
            ),
            (true, true),
            "erase-to-end must follow the body, or a shorter board leaves stale rows"
        );
    }

    #[test]
    fn the_cursor_is_hidden_while_watching_and_given_back_on_the_way_out() {
        let out = written(true, "board", "Next update in 5 seconds");
        assert!(
            out.starts_with(HIDE_CURSOR),
            "hidden before anything is drawn"
        );
        assert!(
            out.ends_with(SHOW_CURSOR),
            "and restored last, so no exit path leaves an invisible caret"
        );
    }

    #[test]
    fn the_status_line_is_erased_before_it_is_rewritten() {
        // "Next update in 9 seconds" over "Next update in 10 seconds" leaves a
        // trailing "s" without the erase, and the countdown reads 9s then 9ss.
        let out = written(true, "board", "Next update in 9 seconds");
        assert_eq!(
            (out.contains('\r'), out.rfind(CLEAR_LINE) > out.rfind('\r')),
            (true, true),
            "rewind, then erase, then write"
        );
    }

    #[test]
    fn not_a_terminal_means_not_one_escape_byte() {
        // A `--watch` that fell back to piped output must write data, not
        // control codes. This is the guard that the fallback path is genuinely
        // free of them rather than merely believed to be.
        let out = written(false, "board", "Next update in 5 seconds");
        assert_eq!(out, "board", "the board, and nothing around it");
        assert!(!out.contains('\u{1b}'), "no escape byte reaches a pipe");
    }

    #[test]
    fn a_countdown_reads_as_updating_once_it_reaches_zero() {
        assert_eq!(countdown(42), "Next update in 42 seconds");
        assert_eq!(countdown(2), "Next update in 2 seconds");
        assert_eq!(countdown(1), "Next update in 1 second", "not `1 seconds`");

        // A poll that overran its interval must not advertise a negative
        // number, which reads as a bug rather than a slow request.
        assert_eq!(countdown(0), updating());
        assert_eq!(countdown(-5), updating());
    }
}
