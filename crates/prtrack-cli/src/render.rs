//! Drawing a decided board as text a terminal can show.
//!
//! Nothing here decides anything. Every judgement on the screen was made in
//! `prtrack-core` and arrives as a [`Board`]; this turns it into columns.

use std::fmt::Write as _;

use chrono::{DateTime, Utc};
use prtrack_core::{
    BlockedReason, BlockingChecks, Board, BoardRow, DraftState, Login, MergeState, MergeableState,
    Placed, PolicyRules, Requirement, Reviewability, Reviewer, StaleReviewHandling,
};

use crate::color::{Palette, Style};
use crate::width::{pad_to, truncate_to};

/// Column widths, in terminal cells. The last column is unbounded, so it has
/// no entry.
const PR_COLUMN: usize = 7;
const AUTHOR_COLUMN: usize = 14;
const CI_COLUMN: usize = 15;
// Nine, not eight, because `pad_to` guarantees at least one trailing space so
// two values can never run together. `OPEN.CMT` is exactly eight characters, so
// at width eight the *heading* rendered nine cells while the data rendered
// eight, and every heading to its right sat one cell off its column. Invisible
// until something followed `SINCE PUSH`.
const THREADS_COLUMN: usize = 9;
const MERGE_COLUMN: usize = 11;
const REASON_COLUMN: usize = 40;
/// Only padded when something follows it. As the last column it needs no width.
const SINCE_PUSH_COLUMN: usize = 11;

/// How many cells each level of stacking adds to the PR column: the indent
/// that pushes a stacked row's number out from under the one it sits on.
const TREE_STEP: usize = 2;

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

/// How much of a branch name is shown before it is cut.
///
/// Last column, so it could run to the edge — but a branch is scanned for
/// recognition rather than read, and an unbounded last column turns a wide
/// terminal into a wall of slugs.
const BRANCH_CELLS: usize = 34;

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
    /// Whether to show the branch each pull request lives on.
    ///
    /// On by default: the branch is what you type to go and look at the thing,
    /// and it is the one field the row otherwise makes you open GitHub for. Off
    /// for anybody whose branch names are long enough to push the row past the
    /// width of their terminal.
    pub show_branch: bool,
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

        // Other people's work first. The board exists to stop somebody being
        // blocked by you, and what you are holding up outranks what you are
        // waiting on — your own rows are still there, one screen-glance lower.
        //
        // Work nobody asked you about is appended rather than hidden: "ready
        // for review, nobody assigned" is the state that stalls longest and it
        // shows up nowhere else. No sub-heading, because each row's reason
        // already says `NOBODY ASSIGNED` or names whoever *was* asked.
        //
        // What you have already reviewed has a section of its own rather than
        // a line in the review list: asking you to review your own approval is
        // the fastest way to teach somebody to stop reading that list, and
        // hiding it lost the pull requests that went bad after you approved.
        // One width for every section, so a stack in one section does not leave
        // the columns of the next a few cells to the left of it.
        let pr_column = PR_COLUMN + self.tree_width();
        let review = self
            .board
            .needs_your_review()
            .walk()
            .into_iter()
            .chain(self.board.unclaimed().walk())
            .collect::<Vec<_>>();
        let sections = [
            self.section("NEED YOUR REVIEW", &review, true, pr_column),
            self.section(
                "YOU REVIEWED",
                &self.board.you_reviewed().walk(),
                true,
                pr_column,
            ),
            self.section("YOUR PRS", &self.board.yours().walk(), true, pr_column),
            // Directly under your open work, because it is the same question
            // asked one step earlier: what have you got going, and what have
            // you not asked for yet.
            self.section(
                "YOUR DRAFTS",
                &self.board.your_drafts().walk(),
                true,
                pr_column,
            ),
            // No column header: nobody scans a draft they do not own for its CI.
            self.section(
                "OTHER DRAFTS",
                &self.board.other_drafts().walk(),
                false,
                pr_column,
            ),
        ];
        for block in sections.into_iter().filter(|block| !block.is_empty()) {
            lines.extend(block);
            lines.push(String::new());
        }
        lines.push(self.summary_line());
        lines.push(self.legend_line());

        let mut rendered = lines.join("\n");
        rendered.push('\n');
        rendered
    }

    /// How much wider the PR column is than [`PR_COLUMN`], so that the deepest
    /// stack on the board still leaves every other column aligned.
    fn tree_width(&self) -> usize {
        self.board
            .walk()
            .map(|placed| placed.depth())
            .max()
            .unwrap_or(0)
            * TREE_STEP
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

    /// A heading, optionally a column header, and the rows — or nothing at all
    /// when there are none: an empty section is noise, and the summary line
    /// still counts every pull request.
    ///
    /// `pr_column` is the PR column's width on this board, which a stack
    /// anywhere on it widens.
    fn section(
        &self,
        title: &str,
        rows: &[Placed<'_>],
        with_columns: bool,
        pr_column: usize,
    ) -> Vec<String> {
        if rows.is_empty() {
            return Vec::new();
        }
        let mut lines = vec![format!(
            "{}{}",
            self.palette.paint(Style::Bold, title),
            self.palette
                .paint(Style::Dim, &format!("  ({})", rows.len()))
        )];
        if with_columns {
            lines.push(self.column_header(pr_column));
        }
        lines.extend(rows.iter().map(|placed| self.row_line(placed, pr_column)));
        lines
    }

    /// The column header, which must be padded by the same rules as a row or
    /// it will not sit above it.
    fn column_header(&self, pr_column: usize) -> String {
        let header = format!(
            "{}{}{}{}{}{}{}",
            pad_to("PR", pr_column),
            pad_to("AUTHOR", AUTHOR_COLUMN),
            pad_to("CI", CI_COLUMN),
            pad_to("OPEN.CMT", THREADS_COLUMN),
            pad_to("MERGE", MERGE_COLUMN),
            pad_to("WHY IT CANNOT MERGE", REASON_COLUMN),
            if self.show_branch {
                pad_to("SINCE PUSH", SINCE_PUSH_COLUMN)
            } else {
                "SINCE PUSH".to_owned()
            }
        );
        let header = if self.show_branch {
            format!("{header}BRANCH")
        } else {
            header
        };
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
    ///
    /// A stacked row's number is indented under the one it sits on, behind a
    /// branch mark; `pr_column` is wide enough for the deepest of them.
    fn row_line(&self, placed: &Placed<'_>, pr_column: usize) -> String {
        let row = placed.row();
        let facts = row.facts();
        let is_draft = facts.draft_state == DraftState::Draft;
        let unreadable = row.reviewability() >= Reviewability::CiRunning;
        let palette = if is_draft || unreadable {
            self.palette.dimmed()
        } else {
            self.palette
        };

        let author = facts.author.as_ref().map_or(DELETED_AUTHOR, Login::as_str);

        let line = format!(
            "{}{}{}{}{}{}{}",
            // The number is a link to the pull request. OSC 8 costs no columns,
            // so the URL is reachable without a column wide enough to print it
            // — and the row stays scannable, which a full URL per row does not.
            // The tree mark sits outside the link: it is not the pull request.
            pad_to(
                &format!(
                    "{}{}",
                    tree_prefix(placed.depth()),
                    palette.link(
                        facts.url.as_str(),
                        &palette.paint(Style::Plain, &format!("#{}", facts.number.get())),
                    )
                ),
                pr_column
            ),
            pad_to(
                &palette.paint(Style::Plain, truncate_to(author, AUTHOR_CELLS)),
                AUTHOR_COLUMN
            ),
            // `self.palette`, not the dimmed one: see the note above. The
            // reason is status too — it is the answer to the question the
            // column asks — so it keeps its colour with the other two.
            pad_to(&checks_cell(row, self.palette), CI_COLUMN),
            pad_to(&threads_cell(row, self.rules, self.palette), THREADS_COLUMN),
            pad_to(&merge_cell(row, is_draft, palette), MERGE_COLUMN),
            pad_to(&reason_cell(row.reason(), self.palette), REASON_COLUMN),
            if self.show_branch {
                pad_to(
                    &palette.paint(Style::Plain, &self.age_of(row)),
                    SINCE_PUSH_COLUMN,
                )
            } else {
                palette.paint(Style::Plain, &self.age_of(row))
            },
        );
        if self.show_branch {
            // Dim in every row, including one that is otherwise bright: the
            // branch is how you reach the work, not a judgement about it, and
            // painting it the same weight as the reason would have the row
            // shouting twice.
            let branch = self.palette.paint(
                Style::Dim,
                truncate_to(facts.head_ref.as_str(), BRANCH_CELLS),
            );
            format!("{line}{branch}")
        } else {
            line
        }
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
        let every_row = || self.board.walk().map(|placed| placed.row());
        let ready = every_row()
            .filter(|row| matches!(row.reason(), BlockedReason::ReadyToMerge))
            .count();
        let conflicting = every_row()
            .filter(|row| {
                row.facts().draft_state == DraftState::ReadyForReview
                    && row.facts().mergeable == MergeableState::Conflicting
            })
            .count();
        // Everything the review section shows that is worth opening now, which
        // includes the unclaimed rows appended to it.
        let reviewable = self
            .board
            .awaiting_review()
            .filter(|row| row.reviewability() < Reviewability::CiRunning)
            .count();

        self.palette.paint(
            Style::Dim,
            &format!(
                // How many pull requests this viewer has already reviewed,
                // counted by each row's own placement, so one stacked under
                // another section still counts. Kept here so the number is
                // visible without reading the YOU REVIEWED section.
                "{} open | {} you have reviewed | {ready} ready to merge | \
                 {conflicting} conflicting | {reviewable} reviewable now",
                self.board.total(),
                self.board.you_have_reviewed(),
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

/// The indent and branch mark in front of a stacked row's number: nothing for a
/// root, `└ ` one level up, and [`TREE_STEP`] more cells of indent per level
/// above that.
fn tree_prefix(depth: usize) -> String {
    depth
        .checked_sub(1)
        .map_or_else(String::new, |levels_below| {
            format!("{}└ ", " ".repeat(levels_below * TREE_STEP))
        })
}

/// What CI says, in the space of one column.
///
/// The order is a ladder: a blocking failure is the only thing worth saying
/// when there is one, and an advisory failure is worth saying only when
/// nothing blocks. An empty rollup is called out rather than drawn as a pass,
/// because "CI never ran" and "CI is green" are the two readings a board must
/// never confuse.
fn checks_cell(row: &BoardRow, palette: Palette) -> String {
    let checks = row.checks();
    let (blocking, advisory) = (checks.blocking().len(), checks.advisory().len());
    let (running, total) = (checks.running().len(), checks.total());
    let passing = checks.passing().len();

    if blocking > 0 {
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
        BoardText, AUTHOR_COLUMN, CI_COLUMN, MERGE_COLUMN, PR_COLUMN, REASON_COLUMN,
        SINCE_PUSH_COLUMN, THREADS_COLUMN,
    };
    use crate::color::{ColorChoice, Environment, Palette};
    use crate::width::visible_width;
    use chrono::{DateTime, TimeDelta, Utc};
    use prtrack_core::{
        Board, BoardRow, BranchName, CheckName, DraftState, HeadRepository, HeadSha, InvalidValue,
        Login, MergeState, MergeableState, PolicyRules, PrFacts, PrNumber, PrTitle, PrUrl, RepoRef,
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
            head_ref: BranchName::new("topic/branch-01"),
            head_repository: HeadRepository::ThisOne,
            base_ref: BranchName::new("main"),
            review_decision: None,
            mergeable: MergeableState::Mergeable,
            merge_state: MergeState::Clean,
            requested_reviewers: Vec::new(),
            changes_requested_by: Vec::new(),
            reviewed_by: Vec::new(),
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
    ///
    /// Without the branch column, which most of these tests predate and none of
    /// them are about. [`render_with_branch`] is the one that exercises it.
    fn render(prs: Vec<PrFacts>, rules: &PolicyRules, viewer: &Login, palette: Palette) -> String {
        render_columns(prs, rules, viewer, palette, false)
    }

    /// The same, with the branch column on.
    fn render_with_branch(
        prs: Vec<PrFacts>,
        rules: &PolicyRules,
        viewer: &Login,
        palette: Palette,
    ) -> String {
        render_columns(prs, rules, viewer, palette, true)
    }

    fn render_columns(
        prs: Vec<PrFacts>,
        rules: &PolicyRules,
        viewer: &Login,
        palette: Palette,
        show_branch: bool,
    ) -> String {
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
            show_branch,
        }
        .render()
    }

    #[test]
    fn the_branch_is_shown_last_and_can_be_turned_off() {
        // The branch is the one field the row otherwise makes you open GitHub
        // for, which is why it is on by default -- and why it is last, since it
        // is what you copy rather than what you scan.
        let rendered = Login::parse("octocat").and_then(|me| {
            let mut pr = open_pr(1, "octocat")?;
            pr.head_ref = BranchName::new("topic/branch-01");
            Ok::<_, InvalidValue>((
                render_with_branch(vec![pr.clone()], &enforced_rule()?, &me, plain()),
                render(vec![pr], &enforced_rule()?, &me, plain()),
            ))
        });

        assert_eq!(
            rendered
                .as_ref()
                .map(|(shown, _)| shown.contains("topic/branch-01")),
            Ok(true),
            "shown by default"
        );
        assert_eq!(
            rendered.as_ref().map(|(shown, _)| shown.contains("BRANCH")),
            Ok(true),
            "and the column is named"
        );
        assert_eq!(
            rendered
                .as_ref()
                .map(|(_, hidden)| hidden.contains("topic/branch-01") || hidden.contains("BRANCH")),
            Ok(false),
            "--no-branch removes the heading as well as the values"
        );
    }

    #[test]
    fn the_branch_column_sits_under_its_heading() {
        // The age column is unpadded when it is last and padded when it is not.
        // Getting that wrong puts every branch one place left of its heading,
        // which is the kind of thing that looks like a rendering bug and is
        // never reported because it is only slightly wrong.
        let rendered = Login::parse("octocat").and_then(|me| {
            let mut pr = open_pr(1, "octocat")?;
            pr.head_ref = BranchName::new("topic/branch-01");
            Ok::<_, InvalidValue>(render_with_branch(
                vec![pr],
                &enforced_rule()?,
                &me,
                plain(),
            ))
        });

        let column_of = |needle: &str| {
            rendered.as_ref().ok().and_then(|text| {
                text.lines()
                    .find(|line| line.contains(needle))
                    .and_then(|line| line.find(needle))
            })
        };
        assert_eq!(
            column_of("BRANCH"),
            column_of("topic/branch-01"),
            "heading and value start in the same column"
        );
    }

    #[test]
    fn every_heading_fits_the_column_it_names() {
        // `pad_to` always leaves at least one trailing space, so a heading as
        // long as its column renders one cell wider than the data beneath it --
        // and every column to its right is then off by one. `OPEN.CMT` at width
        // eight did exactly that, and it went unnoticed because the drift only
        // shows once something follows the last column.
        for (heading, column) in [
            ("PR", PR_COLUMN),
            ("AUTHOR", AUTHOR_COLUMN),
            ("CI", CI_COLUMN),
            ("OPEN.CMT", THREADS_COLUMN),
            ("MERGE", MERGE_COLUMN),
            ("WHY IT CANNOT MERGE", REASON_COLUMN),
            ("SINCE PUSH", SINCE_PUSH_COLUMN),
        ] {
            assert!(
                heading.len() < column,
                "{heading:?} needs a column wider than {} to leave a space",
                heading.len()
            );
        }
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
            Ok("#1     octocat       no checks      0.0      clean      \
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
            Ok("#2     日本語        no checks      0.0      clean      \
                NOBODY ASSIGNED                         0m"
                .to_owned())
        );
        // And the two rows therefore occupy the same number of cells, which
        // they cannot if either was padded by character count.
        assert_eq!(
            one_row("octocat", 1).and_then(|ascii| one_row("日本語", 2)
                .map(|wide| (visible_width(&ascii), visible_width(&wide)))),
            Ok((98, 98))
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
            Ok("#1     a-very-long-l no checks      0.0      clean      \
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
        // Three reviewable, not two: the unclaimed row is on screen now and
        // counts toward what somebody could pick up this minute.
        // The count comes from the whole board and not from what was drawn
        // under each heading: a stacked row is drawn beneath its parent's
        // section, and it still has to count where it belongs.
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
                "5 open | 0 you have reviewed | 1 ready to merge | 1 conflicting | 3 reviewable now"
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
    fn a_section_with_nothing_in_it_is_not_printed() {
        let rendered = Login::parse("octocat")
            .and_then(|me| {
                Ok::<_, InvalidValue>(render(
                    vec![open_pr(101, "hubot")?],
                    &PolicyRules::unprotected(),
                    &me,
                    plain(),
                ))
            })
            .unwrap_or_default();
        assert!(rendered.contains("NEED YOUR REVIEW  (1)"), "{rendered}");
        for absent in [
            "YOUR PRS",
            "YOUR DRAFTS",
            "OTHER DRAFTS",
            "YOU REVIEWED",
            "nothing open",
            "  none",
        ] {
            assert!(
                !rendered.contains(absent),
                "{absent:?} printed:\n{rendered}"
            );
        }
        assert!(
            !rendered.contains("\n\n\n"),
            "no run of blank lines where a section was:\n{rendered}"
        );
        // The summary still tells an empty board from a failed read: the
        // failure exits non-zero and never reaches here.
        assert!(rendered.contains("1 open"), "{rendered}");
    }

    /// Every heading a section can print, for asserting that none was.
    const SECTION_HEADINGS: [&str; 5] = [
        "NEED YOUR REVIEW",
        "YOU REVIEWED",
        "YOUR PRS",
        "OTHER DRAFTS",
        "YOUR DRAFTS",
    ];

    /// The first line of each paragraph of a render, a paragraph being what a
    /// blank line separates. Splitting on exactly one blank line means a run of
    /// two shows up as a paragraph whose first line is empty.
    fn paragraph_openers(rendered: &str) -> Vec<&str> {
        rendered
            .trim_end_matches('\n')
            .split("\n\n")
            .map(|paragraph| paragraph.lines().next().unwrap_or_default())
            .collect()
    }

    /// Three kept sections around two empty ones: YOU REVIEWED between the
    /// first two and YOUR DRAFTS between the last two.
    fn three_sections_two_gaps() -> Result<String, InvalidValue> {
        let me = Login::parse("octocat")?;
        let mut theirs = open_pr(3, "hubot")?;
        theirs.requested_reviewers = vec![Reviewer::User(me.clone())];
        let mut their_draft = open_pr(4, "hubot")?;
        their_draft.draft_state = DraftState::Draft;

        Ok(render(
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
    }

    #[test]
    fn an_empty_board_prints_no_section_and_still_answers() {
        let rendered = Login::parse("octocat")
            .map(|me| render(Vec::new(), &PolicyRules::unprotected(), &me, plain()))
            .unwrap_or_default();
        for heading in SECTION_HEADINGS {
            assert!(
                !rendered.contains(heading),
                "{heading:?} printed:\n{rendered}"
            );
        }
        assert!(!rendered.contains("\n\n\n"), "{rendered}");
        // The policy line, one blank line, then the summary: an empty board is
        // visibly an answer and not a blank screen.
        let openers = paragraph_openers(&rendered);
        assert_eq!(openers.len(), 2, "{rendered}");
        assert!(
            openers
                .last()
                .is_some_and(|line| line.starts_with("0 open | ")),
            "{rendered}"
        );
    }

    #[test]
    fn kept_sections_are_separated_by_exactly_one_blank_line() {
        let rendered = three_sections_two_gaps().unwrap_or_default();
        assert!(!rendered.contains("\n\n\n"), "{rendered}");
        let openers = paragraph_openers(&rendered);
        assert!(
            openers
                .first()
                .is_some_and(|line| line.starts_with("prtrack ")),
            "{rendered}"
        );
        assert_eq!(
            openers.get(1..),
            Some(
                [
                    "NEED YOUR REVIEW  (1)",
                    "YOUR PRS  (2)",
                    "OTHER DRAFTS  (1)",
                    "4 open | 0 you have reviewed | 0 ready to merge | 0 conflicting \
                     | 1 reviewable now",
                ]
                .as_slice()
            ),
            "{rendered}"
        );
    }

    #[test]
    fn a_section_heading_counts_the_rows_beneath_it() {
        let rendered = three_sections_two_gaps();

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

    #[test]
    fn a_pull_request_you_reviewed_stays_on_the_board_with_its_blocker() {
        let rendered = Login::parse("octocat")
            .and_then(|me| {
                let mut reviewed = open_pr(101, "hubot")?;
                reviewed.reviewed_by = vec![me.clone()];
                reviewed.mergeable = MergeableState::Conflicting;
                Ok(render(
                    vec![reviewed],
                    &PolicyRules::unprotected(),
                    &me,
                    plain(),
                ))
            })
            .unwrap_or_default();
        assert!(rendered.contains("YOU REVIEWED  (1)"), "{rendered}");
        let row = rendered
            .lines()
            .find(|line| line.starts_with("#101 "))
            .unwrap_or_default();
        assert!(row.contains("CONFLICT"), "{rendered}");
    }

    /// The pull requests on the stacked board, in the order a reader sees them:
    /// #101 at the root, #102 and #103 stacked one and two deep, and #104 on
    /// its own.
    const STACKED_NUMBERS: [&str; 4] = ["#101", "#102", "#103", "#104"];

    /// A three-deep stack and one unstacked pull request, drawn with `palette`.
    /// Handed over out of order, so the tree is the renderer's doing.
    fn stacked_board(palette: Palette) -> String {
        Login::parse("octocat")
            .and_then(|me| {
                let on = |n: u32, head: &str, base: &str| {
                    open_pr(n, "hubot").map(|mut pr| {
                        pr.head_ref = BranchName::new(head);
                        pr.base_ref = BranchName::new(base);
                        pr
                    })
                };
                Ok(render(
                    vec![
                        on(103, "stack/three", "stack/two")?,
                        on(101, "stack/one", "main")?,
                        on(102, "stack/two", "stack/one")?,
                        open_pr(104, "monalisa")?,
                    ],
                    &PolicyRules::unprotected(),
                    &me,
                    palette,
                ))
            })
            .unwrap_or_default()
    }

    /// Asserts every row of [`stacked_board`] starts its author column where
    /// the header does, painted or not.
    ///
    /// Measured in cells with [`visible_width`], not bytes: `└` is three bytes
    /// and one cell, and a colour escape or a link's URL is many bytes and no
    /// cells, so a byte offset would call an aligned row misaligned.
    fn assert_stack_aligned(rendered: &str) {
        let cells_before = |line: &str, at: usize| line.get(..at).map(visible_width);
        let author_at = |line: &str| {
            line.find("hubot")
                .or_else(|| line.find("monalisa"))
                .and_then(|at| cells_before(line, at))
        };
        let header = rendered
            .lines()
            .find(|line| line.contains("AUTHOR"))
            .unwrap_or_default();
        let header_author = header
            .find("AUTHOR")
            .and_then(|at| cells_before(header, at));
        assert!(header_author.is_some(), "{rendered}");
        for n in STACKED_NUMBERS {
            let row = rendered
                .lines()
                .find(|line| line.contains(n))
                .unwrap_or_default();
            assert_eq!(author_at(row), header_author, "{n} misaligned:\n{rendered}");
        }
    }

    #[test]
    fn a_stack_is_drawn_under_the_pull_request_it_sits_on() {
        let rendered = stacked_board(plain());
        let lines: Vec<&str> = rendered.lines().collect();
        let line_of = |needle: &str| lines.iter().position(|line| line.contains(needle));

        assert!(
            lines.iter().any(|line| line.starts_with("#101 ")),
            "{rendered}"
        );
        assert!(
            lines.iter().any(|line| line.starts_with("└ #102 ")),
            "{rendered}"
        );
        assert!(
            lines.iter().any(|line| line.starts_with("  └ #103 ")),
            "{rendered}"
        );
        assert!(
            line_of("#101 ") < line_of("#102 ") && line_of("#102 ") < line_of("#103 "),
            "{rendered}"
        );
        assert_stack_aligned(&rendered);
    }

    #[test]
    fn a_stack_stays_aligned_when_painted_and_linked() {
        // Colour and OSC 8 add bytes and no cells to the PR column, which is
        // exactly where the tree mark adds cells. Both have to be measured the
        // same way or a stacked row shears only on a real terminal.
        let rendered = stacked_board(painting());
        assert!(rendered.contains("\u{1b}]8;;"), "links are on:\n{rendered}");
        assert_stack_aligned(&rendered);

        // The tree mark sits outside the link: what the link draws is the
        // pull request's number and nothing else.
        for n in STACKED_NUMBERS {
            let linked = rendered
                .lines()
                .find(|line| line.contains(n))
                .and_then(|row| row.split("\u{1b}]8;;").nth(1))
                .and_then(|opened| opened.split_once("\u{1b}\\"))
                .map(|(_url, text)| visible_width(text));
            assert_eq!(linked, Some(n.len()), "{n} link text:\n{rendered}");
        }
    }
}
