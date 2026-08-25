//! Drawing a decided board as text a terminal can show.
//!
//! Nothing here decides anything. Every judgement on the screen was made in
//! `prtrack-core` and arrives as a [`Board`]; this turns it into columns.

use std::fmt::Write as _;

use chrono::{DateTime, Utc};
use prtrack_core::{
    BlockedReason, BlockingChecks, Board, BoardRow, DraftState, Login, MergeState, MergeableState,
    PolicyRules, Requirement, Reviewability, Reviewer, StaleReviewHandling,
};

use crate::color::{Palette, Style};
use crate::width::{pad_to, truncate_to};

/// Column widths, in terminal cells. The last column is unbounded, so it has
/// no entry.
const PR_COLUMN: usize = 7;
const AUTHOR_COLUMN: usize = 14;
const CI_COLUMN: usize = 15;
const THREADS_COLUMN: usize = 8;
const MERGE_COLUMN: usize = 11;
const REASON_COLUMN: usize = 40;

/// How much of a login is shown. One cell short of its column, so the longest
/// login still leaves a space before the next value.
const AUTHOR_CELLS: usize = AUTHOR_COLUMN - 1;

/// How an author with no account is written. GitHub reports a null author for
/// a deleted account; the row is still real and still needs a name in the
/// column, but it must not be a name anybody could register.
const DELETED_AUTHOR: &str = "(deleted)";

/// Past this many requested reviewers the list is summarised, because the
/// column cannot hold them and a truncated list reads as the whole list.
const REVIEWERS_BEFORE_SUMMARY: usize = 3;

/// How many are named when it is summarised.
const REVIEWERS_NAMED: usize = 2;

/// Seconds in the units the age column steps through.
const SECONDS_PER_MINUTE: i64 = 60;
const SECONDS_PER_HOUR: i64 = 60 * SECONDS_PER_MINUTE;
const SECONDS_PER_DAY: i64 = 24 * SECONDS_PER_HOUR;

/// A board, and everything needed to draw it.
///
/// Fields are public because this is a plain bundle of arguments: a
/// constructor taking five of them in a row is easier to get wrong than a
/// struct literal that names each.
#[derive(Debug, Clone, Copy)]
pub struct BoardText<'a> {
    /// The decided board.
    pub board: &'a Board,
    /// The rule its rows were judged against, for the policy line.
    pub rules: &'a PolicyRules,
    /// The branch that rule protects, named in the policy line.
    pub default_branch: &'a str,
    /// The instant every age on the screen is measured back from. Passed in
    /// rather than read, so a render is reproducible.
    pub now: DateTime<Utc>,
    /// Whether, and how, to paint.
    pub palette: Palette,
}

impl BoardText<'_> {
    /// Draws the whole board.
    ///
    /// The policy line comes first, before any row, because every judgement
    /// below it was made against those rules and a reader who cannot see them
    /// has no way to know whether a red cell is anybody's fault.
    #[must_use]
    pub fn render(&self) -> String {
        let mut lines: Vec<String> = vec![self.policy_line(), String::new()];

        lines.extend(self.section("YOUR PRS", self.board.yours(), "nothing open", true));
        lines.push(String::new());
        lines.extend(self.section(
            "NEED YOUR REVIEW",
            self.board.needs_your_review(),
            "nothing waiting on you",
            true,
        ));
        lines.push(String::new());
        // No column header: nobody scans a draft they do not own for its CI.
        lines.extend(self.section("OTHER DRAFTS", self.board.other_drafts(), "none", false));
        lines.push(String::new());
        lines.push(self.summary_line());
        lines.push(self.legend_line());

        let mut rendered = lines.join("\n");
        rendered.push('\n');
        rendered
    }

    /// What the repository enforces on the branch these rows target.
    fn policy_line(&self) -> String {
        let mut clauses = vec![match &self.rules.blocking_checks {
            BlockingChecks::Only { required, .. } => {
                format!("{} required checks", required.as_slice().len())
            }
            // Said out loud because the intuitive reading is the opposite of
            // the truth: no required list means we cannot tell which failures
            // matter, so every one of them is painted as blocking below.
            BlockingChecks::Every => {
                "NO required checks - every failure treated as blocking".to_owned()
            }
        }];
        clauses.push(format!("{} approval(s)", self.rules.required_approvals));
        if self.rules.conversation_resolution == Requirement::Required {
            clauses.push("threads must be resolved".to_owned());
        }
        // Both are here because a row below is otherwise unexplained: a
        // code-owner rule is why a review can be requested of a team nobody is
        // named in, and an up-to-date rule is the only thing that makes
        // "behind" a reason rather than a remark.
        if self.rules.code_owner_reviews == Requirement::Required {
            clauses.push("a code owner must approve".to_owned());
        }
        if self.rules.up_to_date_branch == Requirement::Required {
            clauses.push("the branch must be up to date".to_owned());
        }
        if self.rules.stale_reviews == StaleReviewHandling::DismissOnPush {
            clauses.push("a push dismisses approvals".to_owned());
        }

        // The version leads the line. It is the answer to "which build am I
        // looking at", which is the first question asked the moment two people
        // compare screens or somebody reports something already fixed.
        self.palette.paint(
            Style::Dim,
            &format!(
                "prtrack {} | policy on {}: {}",
                crate::update::running(),
                self.default_branch,
                clauses.join(" | ")
            ),
        )
    }

    /// A heading, optionally a column header, and either rows or a note that
    /// there are none.
    fn section(
        &self,
        title: &str,
        rows: &[BoardRow],
        when_empty: &str,
        with_columns: bool,
    ) -> Vec<String> {
        let mut lines = vec![format!(
            "{}{}",
            self.palette.paint(Style::Bold, title),
            self.palette
                .paint(Style::Dim, &format!("  ({})", rows.len()))
        )];
        if with_columns {
            lines.push(self.column_header());
        }
        if rows.is_empty() {
            lines.push(self.palette.paint(Style::Dim, &format!("  {when_empty}")));
        } else {
            lines.extend(rows.iter().map(|row| self.row_line(row)));
        }
        lines
    }

    /// The column header, which must be padded by the same rules as a row or
    /// it will not sit above it.
    fn column_header(&self) -> String {
        let header = format!(
            "{}{}{}{}{}{}{}",
            pad_to("PR", PR_COLUMN),
            pad_to("AUTHOR", AUTHOR_COLUMN),
            pad_to("CI", CI_COLUMN),
            pad_to("OPEN.CMT", THREADS_COLUMN),
            pad_to("MERGE", MERGE_COLUMN),
            pad_to("WHY IT CANNOT MERGE", REASON_COLUMN),
            "SINCE PUSH"
        );
        self.palette.paint(Style::Dim, &header)
    }

    /// One pull request as a row of columns.
    ///
    /// A row the viewer cannot act on is drawn dim — see [`Palette::dimmed`] —
    /// **except its status cells**. CI, open threads and the reason keep their
    /// colour in every row, because the two say different things: the dim means
    /// "you cannot act on this", and the colour means "this is what is wrong".
    /// A dimmed row is still the fastest place to see that six required checks
    /// are red, and flattening that to grey hid the one fact the column exists
    /// to carry.
    ///
    /// What stays dim is identity and timing — the number, the author, the age
    /// — which is enough to read the row as muted without hiding its state.
    ///
    /// Each cell is padded after it is painted, using the width it *shows*
    /// rather than the bytes it holds.
    fn row_line(&self, row: &BoardRow) -> String {
        let facts = row.facts();
        let is_draft = facts.draft_state == DraftState::Draft;
        let unreadable = row.reviewability() >= Reviewability::CiRunning;
        let palette = if is_draft || unreadable {
            self.palette.dimmed()
        } else {
            self.palette
        };

        let author = facts.author.as_ref().map_or(DELETED_AUTHOR, Login::as_str);

        format!(
            "{}{}{}{}{}{}{}",
            // The number is a link to the pull request. OSC 8 costs no columns,
            // so the URL is reachable without a column wide enough to print it
            // — and the row stays scannable, which a full URL per row does not.
            pad_to(
                &palette.link(
                    facts.url.as_str(),
                    &palette.paint(Style::Plain, &format!("#{}", facts.number.get())),
                ),
                PR_COLUMN
            ),
            pad_to(
                &palette.paint(Style::Plain, truncate_to(author, AUTHOR_CELLS)),
                AUTHOR_COLUMN
            ),
            // `self.palette`, not the dimmed one: see the note above. The
            // reason is status too — it is the answer to the question the
            // column asks — so it keeps its colour with the other two.
            pad_to(&checks_cell(row, is_draft, self.palette), CI_COLUMN),
            pad_to(&threads_cell(row, self.rules, self.palette), THREADS_COLUMN),
            pad_to(&merge_cell(row, is_draft, palette), MERGE_COLUMN),
            pad_to(&reason_cell(row.reason(), self.palette), REASON_COLUMN),
            palette.paint(Style::Plain, &self.age_of(row)),
        )
    }

    /// How long ago the head commit landed, to one unit.
    ///
    /// A commit dated after `now` reads as `0m` rather than as a negative age:
    /// GitHub's timestamps come from contributors' clocks, and a skewed one is
    /// not worth a column of its own.
    fn age_of(&self, row: &BoardRow) -> String {
        let seconds = (self.now - row.facts().last_push).num_seconds().max(0);
        if seconds < SECONDS_PER_HOUR {
            format!("{}m", seconds / SECONDS_PER_MINUTE)
        } else if seconds < SECONDS_PER_DAY {
            format!("{}h", seconds / SECONDS_PER_HOUR)
        } else {
            format!("{}d", seconds / SECONDS_PER_DAY)
        }
    }

    /// The tallies under the board.
    fn summary_line(&self) -> String {
        let every_row = || {
            self.board
                .yours()
                .iter()
                .chain(self.board.needs_your_review())
                .chain(self.board.other_drafts())
                .chain(self.board.not_waiting_on_you())
        };
        let ready = every_row()
            .filter(|row| matches!(row.reason(), BlockedReason::ReadyToMerge))
            .count();
        let conflicting = every_row()
            .filter(|row| {
                row.facts().draft_state == DraftState::ReadyForReview
                    && row.facts().mergeable == MergeableState::Conflicting
            })
            .count();
        let reviewable = self
            .board
            .needs_your_review()
            .iter()
            .filter(|row| row.reviewability() < Reviewability::CiRunning)
            .count();

        self.palette.paint(
            Style::Dim,
            &format!(
                "{} open | {} not your review | {ready} ready to merge | \
                 {conflicting} conflicting | {reviewable} reviewable now",
                self.board.total(),
                self.board.not_waiting_on_you().len(),
            ),
        )
    }

    /// How to read the columns that are not self-explanatory.
    fn legend_line(&self) -> String {
        self.palette.paint(
            Style::Dim,
            "CI: x=blocking, !=advisory (runs, cannot block), ~=running   \
             OPEN.CMT=unresolved threads.comments   MERGE shown only once CI is green",
        )
    }
}

/// What CI says, in the space of one column.
///
/// The order is a ladder: a blocking failure is the only thing worth saying
/// when there is one, and an advisory failure is worth saying only when
/// nothing blocks. An empty rollup is called out rather than drawn as a pass,
/// because "CI never ran" and "CI is green" are the two readings a board must
/// never confuse.
fn checks_cell(row: &BoardRow, is_draft: bool, palette: Palette) -> String {
    let checks = row.checks();
    let (blocking, advisory) = (checks.blocking().len(), checks.advisory().len());
    let (running, total) = (checks.running().len(), checks.total());
    let passing = checks.passing().len();

    if is_draft {
        palette.paint(Style::Dim, "draft")
    } else if blocking > 0 {
        let mut cell = palette.paint(Style::Red, &format!("x {blocking}"));
        if advisory > 0 {
            // Coloured, not grey: the `+1` is a real failure. Lighter than the
            // blocking count beside it so the eye still lands on the number
            // that decides whether this merges.
            cell.push_str(&palette.paint(Style::LightRed, &format!("+{advisory}")));
        }
        cell
    } else if advisory > 0 {
        palette.paint(Style::LightRed, &format!("! {advisory} adv"))
    } else if running > 0 {
        palette.paint(Style::Orange, &format!("~ {running}/{total} run"))
    } else if total == 0 {
        palette.paint(Style::Dim, "no checks")
    } else {
        palette.paint(Style::Green, &format!("ok {passing}/{total}"))
    }
}

/// Unresolved threads and the comment count, as `threads.comments`.
///
/// The thread count is only red where the repository actually requires
/// resolution; elsewhere an open thread is a conversation, not a blocker, and
/// painting it as one would invent a reason the merge button does not agree
/// with.
fn threads_cell(row: &BoardRow, rules: &PolicyRules, palette: Palette) -> String {
    let facts = row.facts();
    let threads = if facts.unresolved_threads == 0 {
        palette.paint(Style::Dim, "0")
    } else {
        let style = if rules.conversation_resolution == Requirement::Required {
            Style::Red
        } else {
            Style::Yellow
        };
        palette.paint(style, &facts.unresolved_threads.to_string())
    };
    format!(
        "{threads}{}",
        palette.paint(Style::Dim, &format!(".{}", facts.comments))
    )
}

/// What the merge button would do, once CI has stopped being the answer.
///
/// Deliberately blank while anything is failing or running: the merge state of
/// a pull request whose checks have not finished is a guess that changes
/// under the reader, and showing it invites acting on it.
fn merge_cell(row: &BoardRow, is_draft: bool, palette: Palette) -> String {
    let facts = row.facts();
    if is_draft || !row.checks().blocking().is_empty() || !row.checks().running().is_empty() {
        return palette.paint(Style::Dim, "-");
    }
    match facts.mergeable {
        MergeableState::Conflicting => palette.paint(Style::Red, "CONFLICT"),
        MergeableState::Computing => palette.paint(Style::Dim, "computing"),
        MergeableState::Mergeable | MergeableState::Unknown { .. } => {
            if facts.merge_state == MergeState::Behind {
                palette.paint(Style::Dim, "behind")
            } else {
                palette.paint(Style::Green, "clean")
            }
        }
    }
}

/// The sentence that says why this pull request cannot merge.
///
/// One arm per rung, with no wildcard, so a rung added to the ladder later
/// cannot render as whatever the arm above it happened to say.
///
/// Public because the JSON projection needs the same sentence unpainted: the
/// acceptance diff compares it as a field, and two spellings of one verdict
/// would be a difference nobody introduced.
#[must_use]
pub fn reason_text(reason: &BlockedReason, palette: Palette) -> String {
    reason_cell(reason, palette)
}

/// The sentence itself.
fn reason_cell(reason: &BlockedReason, palette: Palette) -> String {
    match reason {
        BlockedReason::Draft => palette.paint(Style::Dim, "draft - not ready"),
        BlockedReason::ChangesRequested { by } => palette.paint(
            Style::Red,
            &format!("changes requested by {}", join_logins(by.as_slice())),
        ),
        BlockedReason::RequiredCheckFailing { .. } => {
            palette.paint(Style::Red, "required check failing")
        }
        BlockedReason::UnresolvedThreads { count } => palette.paint(
            Style::Magenta,
            &format!("{} unresolved thread(s)", count.get()),
        ),
        BlockedReason::ApprovedButConflicting => {
            palette.paint(Style::Red, "approved - REBASE NEEDED")
        }
        BlockedReason::Conflicting => palette.paint(Style::Red, "conflicting - rebase first"),
        BlockedReason::BehindBase => palette.paint(Style::Yellow, "approved - UPDATE BRANCH"),
        BlockedReason::ApprovedCiRunning => palette.paint(Style::Orange, "approved - CI running"),
        BlockedReason::ReadyToMerge => palette.paint(Style::Green, "READY TO MERGE"),
        BlockedReason::CiRunning => palette.paint(Style::Orange, "CI running"),
        BlockedReason::MergeabilityNotKnown => palette.paint(Style::Dim, "mergeability not known"),
        BlockedReason::WaitingForReview { from } => palette.paint(
            Style::Blue,
            &format!("review: {}", name_reviewers(from.as_slice())),
        ),
        BlockedReason::NobodyAssigned => palette.paint(Style::Blue, "NOBODY ASSIGNED"),
    }
}

/// Joins account names for the changes-requested sentence.
fn join_logins(logins: &[Login]) -> String {
    logins
        .iter()
        .map(Login::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Names who a review is waiting on, summarising once the list outgrows the
/// column.
///
/// **Every request is either named or counted.** A reviewer of a kind this
/// crate cannot name is still somebody who was asked, and leaving them out
/// makes the sentence report fewer people than are actually blocking the pull
/// request — a pull request waiting on three reads as waiting on two. That is
/// the same fault as the `NOBODY ASSIGNED` bug already fixed in the oracle,
/// where a dropped request made a reviewed pull request look unassigned; this
/// is the half of it that was left.
///
/// So the summarising threshold counts *all* requests, while the overflow
/// `+n` counts only the ones that have a name and were not listed. Anything
/// that cannot be named is said separately, because "two more people" and "two
/// requests nobody can be named for" are different facts.
///
/// ```text
/// alice, bob, carol            three requests, all named
/// alice, bob +2                four requests, all nameable, two not listed
/// alice, bob, 1 unnamed        three requests, one of them unnameable
/// alice, bob +2, 1 unnamed     five requests, four nameable, one not
/// 2 unnamed reviewer(s)        two requests, neither nameable
/// ```
///
/// The wording is load-bearing beyond the screen: this sentence is a field in
/// the JSON projection, so a script may match on it. Change it deliberately,
/// not incidentally.
fn name_reviewers(reviewers: &[Reviewer]) -> String {
    let names: Vec<&str> = reviewers.iter().filter_map(Reviewer::name).collect();
    let unnamed = reviewers.len() - names.len();

    let shown: Vec<&str> = if reviewers.len() > REVIEWERS_BEFORE_SUMMARY {
        names.iter().take(REVIEWERS_NAMED).copied().collect()
    } else {
        names.clone()
    };

    if shown.is_empty() {
        // Every request is of a kind with no name. Reached only through a
        // non-empty list, since the rung that calls this fires on one.
        return format!("{unnamed} unnamed reviewer(s)");
    }

    let mut sentence = shown.join(", ");
    let nameable_but_unlisted = names.len() - shown.len();
    if nameable_but_unlisted > 0 {
        // Writing into the string cannot fail, and the result is discarded
        // rather than unwrapped so that no panic path exists here at all.
        let _ = write!(sentence, " +{nameable_but_unlisted}");
    }
    if unnamed > 0 {
        let _ = write!(sentence, ", {unnamed} unnamed");
    }
    sentence
}

#[cfg(test)]
mod tests {
    use super::{
        BoardText, AUTHOR_COLUMN, CI_COLUMN, MERGE_COLUMN, PR_COLUMN, REASON_COLUMN, THREADS_COLUMN,
    };
    use crate::color::{ColorChoice, Environment, Palette};
    use crate::width::visible_width;
    use chrono::{DateTime, TimeDelta, Utc};
    use prtrack_core::{
        Board, BoardRow, BranchName, CheckName, DraftState, HeadSha, InvalidValue, Login,
        MergeState, MergeableState, PolicyRules, PrFacts, PrNumber, PrTitle, PrUrl, RepoRef,
        RequiredChecks, Requirement, Reviewer, StaleReviewHandling,
    };

    /// An object id. Nothing rendered reads it.
    const SOME_SHA: &str = "9f2c4e1b7a3d5068c9e4b2a17f36d8095c1e4a7b";

    /// An instant `minutes` from an arbitrary base.
    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::<Utc>::UNIX_EPOCH + TimeDelta::minutes(minutes)
    }

    /// A palette that paints, so a test can assert on the escapes themselves.
    fn painting() -> Palette {
        Palette::resolve(
            ColorChoice::Always,
            &Environment {
                no_color: None,
                clicolor: None,
                stdout_is_terminal: true,
            },
        )
    }

    /// A palette that never paints, so a test can read the layout.
    fn plain() -> Palette {
        Palette::resolve(
            ColorChoice::Never,
            &Environment {
                no_color: None,
                clicolor: None,
                stdout_is_terminal: true,
            },
        )
    }

    /// An open pull request with nothing wrong with it.
    fn open_pr(number: u32, author: &str) -> Result<PrFacts, InvalidValue> {
        Ok(PrFacts {
            repo: RepoRef::parse("octo-org/example")?,
            number: PrNumber::new(number)?,
            url: PrUrl::new("https://github.com/octo-org/example/pull/1"),
            title: PrTitle::new("Split the ledger writer"),
            author: Some(Login::parse(author)?),
            draft_state: DraftState::ReadyForReview,
            head: HeadSha::parse(SOME_SHA)?,
            base_ref: BranchName::new("main"),
            review_decision: None,
            mergeable: MergeableState::Mergeable,
            merge_state: MergeState::Clean,
            requested_reviewers: Vec::new(),
            changes_requested_by: Vec::new(),
            unresolved_threads: 0,
            comments: 0,
            checks: Vec::new(),
            last_push: at(0),
        })
    }

    /// The rule `octo-org/example` enforces on `main`.
    fn enforced_rule() -> Result<PolicyRules, InvalidValue> {
        let contexts = ["Preview CI Gate", "Validate PR Title"]
            .iter()
            .map(|name| CheckName::parse(name))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(PolicyRules {
            blocking_checks: prtrack_core::BlockingChecks::from_rule(
                Requirement::Required,
                contexts,
            ),
            required_approvals: 1,
            conversation_resolution: Requirement::Required,
            code_owner_reviews: Requirement::Required,
            up_to_date_branch: Requirement::NotRequired,
            stale_reviews: StaleReviewHandling::DismissOnPush,
        })
    }

    /// Renders a board built from `prs` under `rules`, seen by `viewer`.
    fn render(prs: Vec<PrFacts>, rules: &PolicyRules, viewer: &Login, palette: Palette) -> String {
        let rows = prs
            .into_iter()
            .map(|facts| BoardRow::reduce(facts, rules))
            .collect();
        BoardText {
            board: &Board::assemble(rows, viewer),
            rules,
            default_branch: "main",
            now: at(0),
            palette,
        }
        .render()
    }

    /// The first line, which is the policy banner.
    fn first_line(rendered: &str) -> String {
        rendered.lines().next().unwrap_or_default().to_owned()
    }

    /// The rendered line for pull request `number` in a painted render, where
    /// the row may open with an escape rather than with the number.
    fn row_for_painted(rendered: &str, number: u32) -> String {
        rendered
            .lines()
            .find(|line| line.contains(&format!("#{number}")))
            .unwrap_or_default()
            .to_owned()
    }

    /// The rendered line for pull request `number`, which must begin at column
    /// zero — so a row that acquired a leading anything fails here.
    fn row_for(rendered: &str, number: u32) -> String {
        rendered
            .lines()
            .find(|line| line.starts_with(&format!("#{number} ")))
            .unwrap_or_default()
            .to_owned()
    }

    #[test]
    fn a_row_puts_every_column_where_the_header_says_it_is() {
        // The exact bytes, because alignment is the one property of a table
        // that cannot be checked by looking at any single cell.
        let rendered = Login::parse("octocat").and_then(|me| {
            Ok::<_, InvalidValue>(render(
                vec![open_pr(1, "octocat")?],
                &enforced_rule()?,
                &me,
                plain(),
            ))
        });

        assert_eq!(
            rendered.as_ref().map(|text| row_for(text, 1)),
            Ok("#1     octocat       no checks      0.0     clean      \
                NOBODY ASSIGNED                         0m"
                .to_owned())
        );
    }

    #[test]
    fn a_wide_glyph_in_a_login_does_not_shift_the_columns_after_it() {
        // A CJK login draws two cells per character. Padding it by character
        // count leaves the cell three cells short and every column to its
        // right slides left — the failure the whole width module exists for.
        //
        // `Login` accepts this: GitHub's own account rules would not produce
        // it, but the renderer must not misalign on input the type permits.
        //
        // Two boards rather than two rows, because a row is only drawn in a
        // section, and the section a pull request lands in depends on who is
        // looking. Same fixture otherwise, so the only difference between the
        // two lines is the login.
        let one_row = |author: &str, number: u32| -> Result<String, InvalidValue> {
            let me = Login::parse(author)?;
            let rendered = render(
                vec![open_pr(number, author)?],
                &enforced_rule()?,
                &me,
                plain(),
            );
            Ok(row_for(&rendered, number))
        };

        // Six cells of login and eight of padding, against seven and seven.
        assert_eq!(
            one_row("日本語", 2),
            Ok("#2     日本語        no checks      0.0     clean      \
                NOBODY ASSIGNED                         0m"
                .to_owned())
        );
        // And the two rows therefore occupy the same number of cells, which
        // they cannot if either was padded by character count.
        assert_eq!(
            one_row("octocat", 1).and_then(|ascii| one_row("日本語", 2)
                .map(|wide| (visible_width(&ascii), visible_width(&wide)))),
            Ok((97, 97))
        );
    }

    #[test]
    fn a_login_too_long_for_its_column_is_cut_rather_than_pushing_the_row() {
        // The only column that truncates. A login is the one value a person
        // chooses the length of, and a 39-character one would push every
        // column on every other row out of line with this one.
        let rendered = Login::parse("a-very-long-login-indeed").and_then(|me| {
            Ok::<_, InvalidValue>(render(
                vec![open_pr(1, "a-very-long-login-indeed")?],
                &enforced_rule()?,
                &me,
                plain(),
            ))
        });

        assert_eq!(
            rendered.as_ref().map(|text| row_for(text, 1)),
            Ok("#1     a-very-long-l no checks      0.0     clean      \
                NOBODY ASSIGNED                         0m"
                .to_owned()),
            "thirteen cells of login and the one space that keeps the columns apart"
        );
    }

    #[test]
    fn an_author_whose_account_is_gone_is_named_as_gone() {
        // GitHub reports a null author for a deleted account. The shell
        // prototype used to write the literal `unknown`, which a real account
        // of that name would collide with; the placeholder here is one nobody
        // can register.
        // Rendered through the review section, because a pull request with no
        // author can never be the viewer's own.
        let rendered = Login::parse("octocat").and_then(|me| {
            let mut orphaned = open_pr(1, "hubot")?;
            orphaned.author = None;
            orphaned.requested_reviewers = vec![Reviewer::User(me.clone())];
            Ok::<_, InvalidValue>(render(vec![orphaned], &enforced_rule()?, &me, plain()))
        });

        assert_eq!(
            rendered
                .as_ref()
                .map(|text| row_for(text, 1).contains("(deleted)")),
            Ok(true)
        );
    }

    #[test]
    fn the_age_column_steps_from_minutes_to_hours_to_days() {
        // One unit, because the column is four cells wide and "3 days, 4
        // hours" is not a thing anybody acts differently on.
        let cases: [(i64, &str); 6] = [
            (0, "0m"),
            (59, "59m"),
            (60, "1h"),
            (1439, "23h"),
            (1440, "1d"),
            (30_240, "21d"),
        ];
        for (minutes_ago, expected) in cases {
            let rendered = Login::parse("octocat").and_then(|me| {
                let mut pushed_then = open_pr(1, "octocat")?;
                pushed_then.last_push = at(-minutes_ago);
                Ok::<_, InvalidValue>(render(vec![pushed_then], &enforced_rule()?, &me, plain()))
            });
            assert_eq!(
                rendered.as_ref().map(|text| age_column(text, 1)),
                Ok(expected.to_owned()),
                "{minutes_ago} minutes ago must read as {expected:?}"
            );
        }
    }

    #[test]
    fn a_commit_dated_in_the_future_reads_as_no_age_rather_than_a_negative_one() {
        // Commit timestamps come from contributors' machines, and a skewed
        // clock is not worth a column of its own. `-3m` would read as a bug in
        // the board rather than a bug in somebody's laptop.
        let rendered = Login::parse("octocat").and_then(|me| {
            let mut from_the_future = open_pr(1, "octocat")?;
            from_the_future.last_push = at(90);
            Ok::<_, InvalidValue>(render(
                vec![from_the_future],
                &enforced_rule()?,
                &me,
                plain(),
            ))
        });

        assert_eq!(
            rendered.as_ref().map(|text| age_column(text, 1)),
            Ok("0m".to_owned())
        );
    }

    /// A rule that demands everything the ladder can ask about, so one board
    /// can reach the rungs that only exist under a strict policy.
    fn demanding_rule() -> Result<PolicyRules, InvalidValue> {
        Ok(PolicyRules {
            up_to_date_branch: Requirement::Required,
            ..enforced_rule()?
        })
    }

    /// The age cell, which is everything past the reason column.
    fn age_column(rendered: &str, number: u32) -> String {
        const AGE_STARTS_AT: usize =
            PR_COLUMN + AUTHOR_COLUMN + CI_COLUMN + THREADS_COLUMN + MERGE_COLUMN + REASON_COLUMN;
        row_for(rendered, number)
            .chars()
            .skip(AGE_STARTS_AT)
            .collect::<String>()
            .trim_end()
            .to_owned()
    }

    /// The reason cell, read out of the row by its column position rather than
    /// by searching for the text — so a sentence that rendered into the wrong
    /// column fails instead of being found anyway.
    fn reason_column(rendered: &str, number: u32) -> String {
        const REASON_STARTS_AT: usize =
            PR_COLUMN + AUTHOR_COLUMN + CI_COLUMN + THREADS_COLUMN + MERGE_COLUMN;
        row_for(rendered, number)
            .chars()
            .skip(REASON_STARTS_AT)
            .take(REASON_COLUMN)
            .collect::<String>()
            .trim_end()
            .to_owned()
    }

    #[test]
    fn every_rung_of_the_ladder_gets_its_own_sentence_in_its_own_column() {
        // One fixture per rung. Two rungs sharing a sentence would make the
        // board lie about which of them fired, and a sentence in the wrong
        // column is a sentence nobody reads.
        let sentences = Login::parse("octocat").and_then(|me| {
            let approved = Some(prtrack_core::ReviewDecision::Approved);
            let failing = prtrack_core::Check::new(
                CheckName::parse("Preview CI Gate")?,
                prtrack_core::CheckState::parse("FAILURE"),
                Some(at(0)),
            );
            let building = prtrack_core::Check::new(
                CheckName::parse("Preview CI Gate")?,
                prtrack_core::CheckState::parse("IN_PROGRESS"),
                Some(at(0)),
            );

            let mut prs = Vec::new();
            let mut push =
                |number: u32, spoil: &dyn Fn(&mut PrFacts)| -> Result<(), InvalidValue> {
                    let mut facts = open_pr(number, "octocat")?;
                    spoil(&mut facts);
                    prs.push(facts);
                    Ok(())
                };

            push(1, &|f| f.draft_state = DraftState::Draft)?;
            let blocker = Login::parse("hubot")?;
            push(2, &|f| f.changes_requested_by = vec![blocker.clone()])?;
            push(3, &|f| f.checks = vec![failing.clone()])?;
            push(4, &|f| f.unresolved_threads = 2)?;
            push(5, &|f| {
                f.review_decision = approved.clone();
                f.mergeable = MergeableState::Conflicting;
            })?;
            push(6, &|f| f.mergeable = MergeableState::Conflicting)?;
            push(7, &|f| {
                f.review_decision = approved.clone();
                f.merge_state = MergeState::Behind;
            })?;
            push(8, &|f| {
                f.review_decision = approved.clone();
                f.checks = vec![building.clone()];
            })?;
            push(9, &|f| f.review_decision = approved.clone())?;
            push(10, &|f| f.checks = vec![building.clone()])?;
            push(11, &|f| f.mergeable = MergeableState::Computing)?;
            let wanted = Login::parse("hubot")?;
            push(12, &|f| {
                f.requested_reviewers = vec![Reviewer::User(wanted.clone())];
            })?;
            push(13, &|_| ())?;

            let rendered = render(prs, &demanding_rule()?, &me, plain());
            Ok::<_, InvalidValue>(
                (1..=13)
                    .map(|number| reason_column(&rendered, number))
                    .collect::<Vec<_>>(),
            )
        });

        assert_eq!(
            sentences,
            Ok(vec![
                "draft - not ready".to_owned(),
                "changes requested by hubot".to_owned(),
                "required check failing".to_owned(),
                "2 unresolved thread(s)".to_owned(),
                "approved - REBASE NEEDED".to_owned(),
                "conflicting - rebase first".to_owned(),
                "approved - UPDATE BRANCH".to_owned(),
                "approved - CI running".to_owned(),
                "READY TO MERGE".to_owned(),
                "CI running".to_owned(),
                "mergeability not known".to_owned(),
                "review: hubot".to_owned(),
                "NOBODY ASSIGNED".to_owned(),
            ])
        );
    }

    #[test]
    fn the_ci_cell_reports_the_worst_thing_first_and_never_calls_silence_green() {
        // The order is the point. An advisory failure beside a blocking one is
        // a footnote, not the headline -- painting them together as "2 failing"
        // is what blamed three authors for checks the merge button ignores.
        // And an empty rollup is called out, because "CI never ran" rendered as
        // "CI is green" is the stale cell this whole project exists to avoid.
        let cells = Login::parse("octocat").and_then(|me| {
            let check = |name: &str, state: &str| -> Result<prtrack_core::Check, InvalidValue> {
                Ok(prtrack_core::Check::new(
                    CheckName::parse(name)?,
                    prtrack_core::CheckState::parse(state),
                    Some(at(0)),
                ))
            };
            let required_failing = check("Preview CI Gate", "FAILURE")?;
            let advisory_failing = check("codecov/patch", "FAILURE")?;
            let passing = check("Validate PR Title", "SUCCESS")?;
            let running = check("build / test", "IN_PROGRESS")?;

            let rollups: [(u32, Vec<prtrack_core::Check>); 6] = [
                (1, vec![]),
                (2, vec![passing.clone()]),
                (3, vec![running.clone(), passing.clone()]),
                (4, vec![advisory_failing.clone(), passing.clone()]),
                (5, vec![required_failing.clone(), passing.clone()]),
                (
                    6,
                    vec![required_failing, advisory_failing, running, passing],
                ),
            ];

            let mut prs = Vec::new();
            for (number, checks) in rollups {
                let mut facts = open_pr(number, "octocat")?;
                facts.checks = checks;
                prs.push(facts);
            }

            let rendered = render(prs, &enforced_rule()?, &me, plain());
            Ok::<_, InvalidValue>(
                (1..=6)
                    .map(|number| {
                        row_for(&rendered, number)
                            .chars()
                            .skip(PR_COLUMN + AUTHOR_COLUMN)
                            .take(CI_COLUMN)
                            .collect::<String>()
                            .trim_end()
                            .to_owned()
                    })
                    .collect::<Vec<_>>(),
            )
        });

        assert_eq!(
            cells,
            Ok(vec![
                "no checks".to_owned(),
                "ok 1/1".to_owned(),
                "~ 1/2 run".to_owned(),
                "! 1 adv".to_owned(),
                "x 1".to_owned(),
                "x 1+1".to_owned(),
            ])
        );
    }

    #[test]
    fn a_row_nobody_can_act_on_is_dim_except_where_it_says_what_is_wrong() {
        // Two meanings share a row and neither may swallow the other. The dim
        // says "you cannot act on this"; the colour in the CI and thread cells
        // says "this is what is wrong". Flattening everything to grey was the
        // first attempt and it hid the fact the column exists to carry -- a row
        // with six failing required checks read as unremarkable.
        //
        // What must not happen is a cell reasserting a colour *and* resetting,
        // which ends the row's dimming early and leaves the rest reading as
        // ordinary. That is why the prose cells are checked separately below.
        let rendered = Login::parse("octocat").and_then(|me| {
            let mut failing = open_pr(1, "octocat")?;
            failing.unresolved_threads = 2;
            failing.checks = vec![prtrack_core::Check::new(
                CheckName::parse("Preview CI Gate")?,
                prtrack_core::CheckState::parse("FAILURE"),
                Some(at(0)),
            )];
            Ok::<_, InvalidValue>(render(vec![failing], &enforced_rule()?, &me, painting()))
        });

        let row = rendered.as_ref().map(|text| row_for_painted(text, 1));
        assert_eq!(
            row.as_ref().map(|line| line.contains("\u{1b}[2m")),
            Ok(true),
            "the row is still dim"
        );

        // The blocking count keeps its red, in a row that is otherwise dim.
        assert_eq!(
            row.as_ref().map(|line| line.contains("\u{1b}[31m")),
            Ok(true),
            "a failing required check stays red even where the row is dim"
        );

        // Identity still reads as muted, which is what makes the row scan as
        // one the viewer can skip.
        assert_eq!(
            row.as_ref().map(|line| line.contains("\u{1b}[2mocto")),
            Ok(true),
            "the author stays dim, so the row still reads as one to skip"
        );
    }

    #[test]
    fn the_review_sentence_names_who_it_can_and_counts_who_it_cannot() {
        // The wording is load-bearing beyond the screen: this sentence is a
        // field in the JSON projection, so a script may match on it. Both the
        // threshold and the overflow count are over the reviewers that *have* a
        // name.
        let sentence_for =
            |number: u32, reviewers: Vec<Reviewer>| -> Result<String, InvalidValue> {
                let me = Login::parse("octocat")?;
                let mut waiting = open_pr(number, "octocat")?;
                waiting.requested_reviewers = reviewers;
                let rendered = render(vec![waiting], &enforced_rule()?, &me, plain());
                Ok(reason_column(&rendered, number))
            };
        let user = |login: &str| -> Result<Reviewer, InvalidValue> {
            Ok(Reviewer::User(Login::parse(login)?))
        };

        assert_eq!(
            user("alice").and_then(|a| sentence_for(1, vec![a])),
            Ok("review: alice".to_owned())
        );
        assert_eq!(
            ["alice", "bob", "carol"]
                .iter()
                .map(|login| user(login))
                .collect::<Result<Vec<_>, _>>()
                .and_then(|asked| sentence_for(2, asked)),
            Ok("review: alice, bob, carol".to_owned()),
            "three still fit, so all three are named"
        );
        assert_eq!(
            ["alice", "bob", "carol", "dave"]
                .iter()
                .map(|login| user(login))
                .collect::<Result<Vec<_>, _>>()
                .and_then(|asked| sentence_for(3, asked)),
            Ok("review: alice, bob +2".to_owned()),
            "four do not, so two are named and the rest counted"
        );
        // Four named beside one that cannot be named. Every request is
        // accounted for: two shown, two more nameable, one that cannot be
        // named -- five in all. Counting only the nameable ones would report
        // this pull request as waiting on four people when it waits on five.
        assert_eq!(
            ["alice", "bob", "carol", "dave"]
                .iter()
                .map(|login| user(login))
                .collect::<Result<Vec<_>, _>>()
                .and_then(|mut asked| {
                    asked.push(Reviewer::unknown("Organization"));
                    sentence_for(6, asked)
                }),
            Ok("review: alice, bob +2, 1 unnamed".to_owned()),
            "five requests: two named, two more nameable, one not"
        );
        // Four requests of which only three can be named. The threshold is
        // over *every* request, not over the nameable ones, so this summarises
        // -- and it is the only shape that tells the two counts apart, since
        // anywhere else they agree. Checked against the jq helper directly,
        // which gives the same sentence.
        assert_eq!(
            ["alice", "bob", "carol"]
                .iter()
                .map(|login| user(login))
                .collect::<Result<Vec<_>, _>>()
                .and_then(|mut asked| {
                    asked.push(Reviewer::unknown("Organization"));
                    sentence_for(8, asked)
                }),
            Ok("review: alice, bob +1, 1 unnamed".to_owned()),
            "four requests summarise even though only three of them have names"
        );
        // Small enough to name everybody who has a name, and the one who has
        // none is still said out loud rather than dropped. This is the case
        // that used to read as "alice, bob" -- a pull request waiting on three
        // reviewers rendered as waiting on two.
        assert_eq!(
            ["alice", "bob"]
                .iter()
                .map(|login| user(login))
                .collect::<Result<Vec<_>, _>>()
                .and_then(|mut asked| {
                    asked.push(Reviewer::unknown("Organization"));
                    sentence_for(7, asked)
                }),
            Ok("review: alice, bob, 1 unnamed".to_owned()),
            "three requests, and the third is not silently dropped"
        );
        assert_eq!(
            sentence_for(4, vec![Reviewer::unknown("Organization"); 2]),
            Ok("review: 2 unnamed reviewer(s)".to_owned()),
            "a request nobody can be named for is still a request"
        );
        // A team has a slug, so it is a name and is treated as one.
        assert_eq!(
            sentence_for(
                5,
                vec![Reviewer::Team(prtrack_core::TeamSlug::new("ledger-owners"))]
            ),
            Ok("review: ledger-owners".to_owned())
        );
    }

    #[test]
    fn the_merge_column_stays_blank_until_ci_has_finished_having_its_say() {
        // What the legend at the foot of the board promises. A mergeability
        // computed while checks are still moving is a guess that changes under
        // the reader, and showing `CONFLICT` beside a running build invites
        // somebody to go and rebase a branch that was about to go green.
        //
        // Every fixture here conflicts, so the merge column is the only thing
        // that differs and a renderer ignoring CI would print `CONFLICT` for
        // all three.
        let merge_column = |number: u32,
                            checks: Vec<prtrack_core::Check>|
         -> Result<String, InvalidValue> {
            const MERGE_STARTS_AT: usize = PR_COLUMN + AUTHOR_COLUMN + CI_COLUMN + THREADS_COLUMN;
            let me = Login::parse("octocat")?;
            let mut conflicting = open_pr(number, "octocat")?;
            conflicting.mergeable = MergeableState::Conflicting;
            conflicting.checks = checks;
            let rendered = render(vec![conflicting], &enforced_rule()?, &me, plain());
            Ok(row_for(&rendered, number)
                .chars()
                .skip(MERGE_STARTS_AT)
                .take(MERGE_COLUMN)
                .collect::<String>()
                .trim_end()
                .to_owned())
        };

        let check = |name: &str, state: &str| -> Result<prtrack_core::Check, InvalidValue> {
            Ok(prtrack_core::Check::new(
                CheckName::parse(name)?,
                prtrack_core::CheckState::parse(state),
                Some(at(0)),
            ))
        };

        assert_eq!(
            check("Preview CI Gate", "SUCCESS").and_then(|green| merge_column(1, vec![green])),
            Ok("CONFLICT".to_owned()),
            "with CI green the conflict is worth acting on"
        );
        assert_eq!(
            check("Preview CI Gate", "IN_PROGRESS")
                .and_then(|running| merge_column(2, vec![running])),
            Ok("-".to_owned()),
            "with a build still running the merge state is not yet news"
        );
        assert_eq!(
            check("Preview CI Gate", "FAILURE").and_then(|failing| merge_column(3, vec![failing])),
            Ok("-".to_owned()),
            "with a required check failing the conflict is not the thing to fix first"
        );
    }

    #[test]
    fn an_open_thread_is_only_red_where_the_repository_requires_it_resolved() {
        // Most repositories do not enforce resolution, and on those an open
        // thread is a conversation rather than a blocker. Painting it red
        // everywhere invents a reason the merge button does not agree with,
        // which is the same mistake as painting every failing check red.
        let threads_cell_of = |rules: &PolicyRules| -> Result<String, InvalidValue> {
            let me = Login::parse("octocat")?;
            let mut talking = open_pr(1, "octocat")?;
            talking.unresolved_threads = 2;
            talking.comments = 7;
            let rendered = render(vec![talking], rules, &me, painting());
            Ok(row_for_painted(&rendered, 1))
        };

        let lenient = PolicyRules {
            conversation_resolution: Requirement::NotRequired,
            ..PolicyRules::unprotected()
        };
        assert_eq!(
            threads_cell_of(&lenient).map(|row| row.contains("\u{1b}[33m2\u{1b}[0m")),
            Ok(true),
            "a rule that does not demand resolution makes an open thread yellow"
        );
        assert_eq!(
            threads_cell_of(&lenient).map(|row| row.contains("\u{1b}[31m")),
            Ok(false),
            "and nothing on that row is red"
        );

        let strict = PolicyRules {
            conversation_resolution: Requirement::Required,
            ..PolicyRules::unprotected()
        };
        assert_eq!(
            threads_cell_of(&strict).map(|row| row.contains("\u{1b}[31m2\u{1b}[0m")),
            Ok(true),
            "a rule that demands resolution makes the same thread red"
        );
    }

    #[test]
    fn the_summary_counts_the_rows_the_sections_do_not_show() {
        // The three sections between them hide everything that is somebody
        // else's problem. The count is the only place a reader learns those
        // pull requests exist at all, so it has to come from the whole board
        // and not from what was drawn.
        let summary = Login::parse("octocat").and_then(|me| {
            let mut mine_ready = open_pr(1, "octocat")?;
            mine_ready.review_decision = Some(prtrack_core::ReviewDecision::Approved);

            let mut theirs_conflicting = open_pr(2, "hubot")?;
            theirs_conflicting.mergeable = MergeableState::Conflicting;
            theirs_conflicting.requested_reviewers = vec![Reviewer::User(me.clone())];

            // A draft that conflicts is not a conflict anybody is waiting on:
            // its author has not asked for anything yet, and counting it would
            // report work as stuck that nobody has offered.
            let mut conflicting_draft = open_pr(5, "hubot")?;
            conflicting_draft.draft_state = DraftState::Draft;
            conflicting_draft.mergeable = MergeableState::Conflicting;

            let rendered = render(
                vec![
                    mine_ready,
                    theirs_conflicting,
                    conflicting_draft,
                    open_pr(3, "hubot")?,
                    open_pr(4, "hubot")?,
                ],
                &enforced_rule()?,
                &me,
                plain(),
            );
            Ok::<_, InvalidValue>(
                rendered
                    .lines()
                    .find(|line| line.contains(" open | "))
                    .unwrap_or_default()
                    .to_owned(),
            )
        });

        assert_eq!(
            summary,
            Ok(
                "5 open | 2 not your review | 1 ready to merge | 1 conflicting | 1 reviewable now"
                    .to_owned()
            )
        );
    }

    #[test]
    fn the_policy_line_comes_first_and_says_what_is_enforced() {
        // It leads because everything under it is a judgement made against
        // these rules. A reader who cannot see which checks can block has no
        // way to know whether a red cell is somebody's problem.
        let rendered = Login::parse("octocat").and_then(|me| {
            Ok::<_, InvalidValue>(first_line(&render(
                vec![open_pr(1, "octocat")?],
                &enforced_rule()?,
                &me,
                plain(),
            )))
        });

        assert_eq!(
            rendered,
            // Built from the same source the renderer reads, so a version bump
            // does not edit three tests.
            Ok(format!(
                "prtrack {} | policy on main: 2 required checks | 1 approval(s) \
                 | threads must be resolved | a code owner must approve \
                 | a push dismisses approvals",
                crate::update::running()
            ))
        );
    }

    #[test]
    fn an_unprotected_branch_says_so_rather_than_reporting_nothing() {
        // The dangerous reading of "no required checks" is "nothing can block".
        // It means the opposite here, and the line has to say which, because
        // every failing check on the rows below is about to be painted red.
        let rendered = Login::parse("octocat").and_then(|me| {
            Ok::<_, InvalidValue>(first_line(&render(
                vec![open_pr(1, "octocat")?],
                &PolicyRules::unprotected(),
                &me,
                plain(),
            )))
        });

        assert_eq!(
            rendered,
            Ok(format!(
                "prtrack {} | policy on main: NO required checks \
                 - every failure treated as blocking | 0 approval(s)",
                crate::update::running()
            ))
        );
    }

    #[test]
    fn nothing_paints_when_the_palette_is_off() {
        // The whole screen, not one cell: a single unpainted helper somewhere
        // would still write escapes into a file someone is diffing.
        let rendered = Login::parse("octocat").and_then(|me| {
            let mut failing = open_pr(1, "octocat")?;
            failing.checks = vec![prtrack_core::Check::new(
                CheckName::parse("Preview CI Gate")?,
                prtrack_core::CheckState::parse("FAILURE"),
                Some(at(0)),
            )];
            let mut draft = open_pr(2, "hubot")?;
            draft.draft_state = DraftState::Draft;

            Ok::<_, InvalidValue>(render(
                vec![failing, draft, open_pr(3, "hubot")?],
                &enforced_rule()?,
                &me,
                plain(),
            ))
        });

        assert_eq!(
            rendered.as_ref().map(|text| text.contains('\u{1b}')),
            Ok(false),
            "no escape byte may reach a plain render"
        );
    }

    #[test]
    fn an_empty_section_says_it_is_empty_rather_than_vanishing() {
        // A heading with nothing under it reads as a rendering failure. Naming
        // the emptiness is the difference between "you have nothing open" and
        // "the board broke".
        let rendered = Login::parse("nobody-here").and_then(|me| {
            Ok::<_, InvalidValue>(render(Vec::new(), &enforced_rule()?, &me, plain()))
        });

        for expected in [
            "YOUR PRS  (0)",
            "  nothing open",
            "NEED YOUR REVIEW  (0)",
            "  nothing waiting on you",
            "OTHER DRAFTS  (0)",
            "  none",
        ] {
            assert_eq!(
                rendered.as_ref().map(|text| text.contains(expected)),
                Ok(true),
                "the empty board must still say {expected:?}"
            );
        }
    }

    #[test]
    fn a_section_heading_counts_the_rows_beneath_it() {
        let rendered = Login::parse("octocat").and_then(|me| {
            let mut theirs = open_pr(3, "hubot")?;
            theirs.requested_reviewers = vec![Reviewer::User(me.clone())];
            let mut their_draft = open_pr(4, "hubot")?;
            their_draft.draft_state = DraftState::Draft;

            Ok::<_, InvalidValue>(render(
                vec![
                    open_pr(1, "octocat")?,
                    open_pr(2, "octocat")?,
                    theirs,
                    their_draft,
                ],
                &enforced_rule()?,
                &me,
                plain(),
            ))
        });

        for expected in [
            "YOUR PRS  (2)",
            "NEED YOUR REVIEW  (1)",
            "OTHER DRAFTS  (1)",
        ] {
            assert_eq!(
                rendered.as_ref().map(|text| text.contains(expected)),
                Ok(true),
                "expected the heading {expected:?}"
            );
        }
    }

    #[test]
    fn the_required_check_count_is_the_rules_own_and_not_a_guess() {
        // Guards the policy line against reporting a count from somewhere
        // other than the rule -- the number of rows, say, which would agree
        // with the rule by accident in most fixtures.
        let counted = enforced_rule().map(|rules| match &rules.blocking_checks {
            prtrack_core::BlockingChecks::Only { required, .. } => {
                RequiredChecks::as_slice(required).len()
            }
            prtrack_core::BlockingChecks::Every => 0,
        });
        assert_eq!(counted, Ok(2), "the fixture rule names two contexts");

        let rendered = Login::parse("octocat").and_then(|me| {
            Ok::<_, InvalidValue>(first_line(&render(
                vec![
                    open_pr(1, "octocat")?,
                    open_pr(2, "octocat")?,
                    open_pr(3, "octocat")?,
                ],
                &enforced_rule()?,
                &me,
                plain(),
            )))
        });
        assert_eq!(
            rendered
                .as_ref()
                .map(|line| line.contains("2 required checks")),
            Ok(true),
            "three rows, two required checks: the line must report the rule"
        );
    }

    #[test]
    fn the_policy_line_is_dim_when_painting_and_nothing_else_on_it_is() {
        // It is context, not news. Painting it like a warning would compete
        // with the rows, which are what the reader is here for.
        let rendered = Login::parse("octocat").and_then(|me| {
            Ok::<_, InvalidValue>(first_line(&render(
                vec![open_pr(1, "octocat")?],
                &PolicyRules::unprotected(),
                &me,
                painting(),
            )))
        });

        assert_eq!(
            rendered,
            Ok(format!(
                "\u{1b}[2mprtrack {} | policy on main: NO required checks \
                 - every failure treated as blocking | 0 approval(s)\u{1b}[0m",
                crate::update::running()
            ))
        );
    }
}
