//! The board as data, for a script rather than a person.
//!
//! Everything here is a decided fact — the reason a pull request cannot merge,
//! the counts behind it, the rule it was judged by — and nothing is a rendering
//! choice. No colour, no column widths, no relative ages, so two runs over
//! unchanged data produce identical bytes.
//!
//! The key set is a contract. A field added or removed breaks a script nobody
//! here can see, so the list is pinned by a test rather than left to whatever
//! the emitter happens to write.

use prtrack_core::{
    BlockedReason, BlockingChecks, Board, BoardRow, DraftState, Login, MergeState, MergeableState,
    PolicyRules, Requirement, ReviewDecision, Reviewer, Stacks,
};
use serde_json::{json, Map, Value};

use crate::color::Palette;
use crate::render::reason_text;

/// The board as `--format json` emits it.
///
/// Fields are public for the same reason [`BoardText`]'s are: this is a bundle
/// of arguments, and naming each at the call site beats getting four in a row
/// right.
///
/// [`BoardText`]: crate::render::BoardText
#[derive(Debug, Clone, Copy)]
pub struct BoardJson<'a> {
    /// The decided board, which supplies the sections and their order.
    pub board: &'a Board,
    /// Every row beside the rule it was judged by.
    ///
    /// The rule travels alongside rather than inside the row because a
    /// [`BoardRow`] deliberately does not keep it — the classification and the
    /// reason are the answers, and a renderer needs no more. This projection
    /// is the exception: `governed` asks about the rule directly and cannot be
    /// recovered from a row, because an ungoverned row and a governed one with
    /// nothing advisory are identical once classified.
    pub judged: &'a [(BoardRow, PolicyRules)],
    /// The branch the repository-level rule protects, or `None` where the
    /// repository has no commits and therefore no default branch.
    ///
    /// An `Option` because JSON can say "absent" and should: a placeholder
    /// string is a value a consumer could mistake for a branch name, and there
    /// is no branch to name.
    pub default_branch: Option<&'a str>,
    /// That rule, for the `policy` object.
    pub default_rule: &'a PolicyRules,
}

impl BoardJson<'_> {
    /// Builds the document.
    #[must_use]
    pub fn to_value(&self) -> Value {
        let mut pull_requests: Vec<&(BoardRow, PolicyRules)> = self.judged.iter().collect();
        // Sorted by number so that a diff between two runs turns on
        // content rather than on the order GitHub happened to return.
        pull_requests.sort_by_key(|(row, _)| row.facts().number.get());

        json!({
            "policy": self.policy(),
            "sections": self.sections(),
            "prs": pull_requests
                .into_iter()
                .map(|(row, rules)| pull_request(row, rules))
                .collect::<Vec<_>>(),
        })
    }

    /// What the repository enforces on its default branch.
    fn policy(&self) -> Value {
        // GitHub's order, kept. `requiredStatusCheckContexts` arrives unsorted
        // and the conversion preserves it, so sorting here would reorder a list
        // whose order is GitHub's answer rather than ours.
        //
        // One asymmetry, latent: `RequiredChecks` keeps the first of any
        // repeated name. GitHub's branch protection will not let the same
        // context be added twice, so nothing turns on it today.
        let required: Vec<&str> = match &self.default_rule.blocking_checks {
            BlockingChecks::Only { required, .. } => required
                .as_slice()
                .iter()
                .map(prtrack_core::CheckName::as_str)
                .collect(),
            // Not an omission. The rule was read and named nothing, which is
            // what makes every failing check blocking on the rows below.
            BlockingChecks::Every => Vec::new(),
        };
        json!({
            "default_branch": self.default_branch,
            "required_checks": required,
            "have_required": !required.is_empty(),
            "required_approvals": self.default_rule.required_approvals,
            "conversation_resolution":
                self.default_rule.conversation_resolution == Requirement::Required,
        })
    }

    /// Which rows landed in which section, in the order the board put them.
    ///
    /// The arrays are ordered, so they carry the sort as well as the
    /// membership — which is what lets a diff catch a sort divergence without
    /// comparing sort keys at all.
    fn sections(&self) -> Value {
        fn listed(rows: &Stacks, section: &str) -> Vec<Value> {
            rows.rows()
                .map(|row| json!({ "n": row.facts().number.get(), "section": section }))
                .collect()
        }
        json!({
            "mine": listed(self.board.yours(), "mine"),
            // Separate from `mine`, and from `drafts` which is everybody
            // else's: a consumer counting what somebody is waiting on wants
            // neither of the other two.
            "mine_drafts": listed(self.board.your_drafts(), "mine_drafts"),
            "review": listed(self.board.needs_your_review(), "review"),
            // Rendered under `review`, but a separate key: a consumer counting
            // what was asked of somebody must not be handed what was not.
            "unclaimed": listed(self.board.unclaimed(), "unclaimed"),
            "drafts": listed(self.board.other_drafts(), "drafts"),
            "neither": listed(self.board.you_reviewed(), "neither"),
        })
    }
}

/// One pull request, with everything the diff compares.
fn pull_request(row: &BoardRow, rules: &PolicyRules) -> Value {
    let facts = row.facts();
    let checks = row.checks();

    let mut named: Vec<&str> = facts
        .requested_reviewers
        .iter()
        .filter_map(Reviewer::name)
        .collect();
    named.sort_unstable();
    let unnamed = facts.requested_reviewers.len() - named.len();

    let mut blockers: Vec<&str> = facts
        .changes_requested_by
        .iter()
        .map(Login::as_str)
        .collect();
    blockers.sort_unstable();

    let mut fields = Map::new();
    fields.insert("n".to_owned(), json!(facts.number.get()));
    fields.insert(
        "author".to_owned(),
        facts
            .author
            .as_ref()
            .map_or(Value::Null, |login| json!(login.as_str())),
    );
    fields.insert(
        "draft".to_owned(),
        json!(facts.draft_state == DraftState::Draft),
    );
    fields.insert("governed".to_owned(), json!(is_governed(rules)));
    fields.insert("fail_blocking".to_owned(), json!(checks.blocking().len()));
    fields.insert("fail_advisory".to_owned(), json!(checks.advisory().len()));
    fields.insert("running".to_owned(), json!(checks.running().len()));
    fields.insert("total".to_owned(), json!(checks.total()));
    fields.insert(
        "unresolved_threads".to_owned(),
        json!(facts.unresolved_threads),
    );
    fields.insert("comments".to_owned(), json!(facts.comments));
    fields.insert(
        "mergeable".to_owned(),
        json!(mergeable_wire(&facts.mergeable)),
    );
    fields.insert(
        "merge_state".to_owned(),
        json!(merge_state_wire(&facts.merge_state)),
    );
    fields.insert(
        "review_decision".to_owned(),
        facts
            .review_decision
            .as_ref()
            .map_or(Value::Null, |decision| {
                json!(review_decision_wire(decision))
            }),
    );
    fields.insert("requested".to_owned(), json!(named));
    fields.insert("unnamed".to_owned(), json!(unnamed));
    fields.insert("changes_requested_by".to_owned(), json!(blockers));
    fields.insert("blocked".to_owned(), json!(plain_reason(row.reason())));
    Value::Object(fields)
}

/// Whether a rule covers this pull request *and* names checks that can block.
///
/// Both halves matter: a rule that enforces nothing leaves every failure
/// blocking, which is the same verdict as no rule at all.
const fn is_governed(rules: &PolicyRules) -> bool {
    matches!(rules.blocking_checks, BlockingChecks::Only { .. })
}

/// GitHub's spelling of a mergeability.
///
/// [`MergeableState::Computing`] goes back out as `UNKNOWN`, which is what
/// GitHub called it — the domain renamed it only because a catch-all named
/// `Unknown` already existed on the same enum.
const fn mergeable_wire(state: &MergeableState) -> &str {
    match state {
        MergeableState::Mergeable => "MERGEABLE",
        MergeableState::Conflicting => "CONFLICTING",
        MergeableState::Computing => "UNKNOWN",
        MergeableState::Unknown { reported, .. } => reported.as_str(),
    }
}

/// GitHub's spelling of a merge state, with the same rename undone.
const fn merge_state_wire(state: &MergeState) -> &str {
    match state {
        MergeState::Behind => "BEHIND",
        MergeState::Blocked => "BLOCKED",
        MergeState::Clean => "CLEAN",
        MergeState::Dirty => "DIRTY",
        MergeState::Draft => "DRAFT",
        MergeState::HasHooks => "HAS_HOOKS",
        MergeState::Unstable => "UNSTABLE",
        MergeState::Undetermined => "UNKNOWN",
        MergeState::Unknown { reported, .. } => reported.as_str(),
    }
}

/// GitHub's spelling of a review decision.
const fn review_decision_wire(decision: &ReviewDecision) -> &str {
    match decision {
        ReviewDecision::Approved => "APPROVED",
        ReviewDecision::ChangesRequested => "CHANGES_REQUESTED",
        ReviewDecision::ReviewRequired => "REVIEW_REQUIRED",
        ReviewDecision::Unknown { reported, .. } => reported.as_str(),
    }
}

/// The reason sentence with no styling, since JSON is data.
///
#[must_use]
pub fn plain_reason(reason: &BlockedReason) -> String {
    reason_text(reason, Palette::plain())
}

#[cfg(test)]
mod tests {
    use super::BoardJson;
    use chrono::{DateTime, TimeDelta, Utc};
    use prtrack_core::{
        BlockingChecks, Board, BoardRow, BranchName, Check, CheckName, CheckState, DraftState,
        HeadSha, InvalidValue, Login, MergeState, MergeableState, PolicyRules, PrFacts, PrNumber,
        PrTitle, PrUrl, RepoRef, Requirement, ReviewDecision, Reviewer, StaleReviewHandling,
        TeamSlug,
    };
    use serde_json::{json, Value};

    const SOME_SHA: &str = "9f2c4e1b7a3d5068c9e4b2a17f36d8095c1e4a7b";

    /// The value at a JSON pointer, or `Null` where the document has nothing
    /// there. Indexing a `Value` directly is a panic path the lint set forbids,
    /// and `Null` is the better failure anyway: the assertion then shows what
    /// was missing instead of unwinding.
    fn node<'a>(value: &'a Value, pointer: &str) -> &'a Value {
        const NOTHING: &Value = &Value::Null;
        value.pointer(pointer).unwrap_or(NOTHING)
    }

    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::<Utc>::UNIX_EPOCH + TimeDelta::minutes(minutes)
    }

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

    fn enforced_rule() -> Result<PolicyRules, InvalidValue> {
        let contexts = ["Preview CI Gate", "Validate PR Title"]
            .iter()
            .map(|name| CheckName::parse(name))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(PolicyRules {
            blocking_checks: BlockingChecks::from_rule(Requirement::Required, contexts),
            required_approvals: 1,
            conversation_resolution: Requirement::Required,
            code_owner_reviews: Requirement::Required,
            up_to_date_branch: Requirement::NotRequired,
            stale_reviews: StaleReviewHandling::DismissOnPush,
        })
    }

    fn check(name: &str, state: &str) -> Result<Check, InvalidValue> {
        Ok(Check::new(
            CheckName::parse(name)?,
            CheckState::parse(state),
            Some(at(0)),
        ))
    }

    /// Projects `prs` as `viewer` sees them, every one judged by `rules`.
    fn project(prs: Vec<PrFacts>, rules: &PolicyRules, viewer: &Login) -> Value {
        let judged: Vec<(BoardRow, PolicyRules)> = prs
            .into_iter()
            .map(|facts| (BoardRow::reduce(facts, rules), rules.clone()))
            .collect();
        let board = Board::assemble(judged.iter().map(|(row, _)| row.clone()).collect(), viewer);
        BoardJson {
            board: &board,
            judged: &judged,
            default_branch: Some("main"),
            default_rule: rules,
        }
        .to_value()
    }

    #[test]
    fn every_pull_request_appears_once_in_number_order_however_it_was_sectioned() {
        // The `prs` array is the whole population, including the rows no
        // section renders. A projection that walked the three visible sections
        // would silently drop them, which is the same disappearance the board
        // itself is built to prevent.
        let numbers = Login::parse("octocat").and_then(|me| {
            let mut theirs = open_pr(2, "hubot")?;
            theirs.requested_reviewers = vec![Reviewer::User(me.clone())];
            let mut their_draft = open_pr(4, "hubot")?;
            their_draft.draft_state = DraftState::Draft;

            let value = project(
                vec![
                    open_pr(3, "hubot")?, // nobody's business but its author's
                    their_draft,
                    theirs,
                    open_pr(1, "octocat")?,
                ],
                &enforced_rule()?,
                &me,
            );
            Ok::<_, InvalidValue>(
                node(&value, "/prs")
                    .as_array()
                    .map_or_else(Vec::new, |prs| {
                        prs.iter()
                            .filter_map(|pr| node(pr, "/n").as_u64())
                            .collect()
                    }),
            )
        });

        assert_eq!(numbers, Ok(vec![1_u64, 2, 3, 4]));
    }

    #[test]
    fn the_sections_list_their_rows_in_the_order_the_board_put_them() {
        // The arrays are ordered, so they carry the sort as well as the
        // membership. That is what makes a sort divergence visible in a diff
        // without the sort keys having to be compared at all.
        let sections = Login::parse("octocat").and_then(|me| {
            let mut mine_draft = open_pr(1, "octocat")?;
            mine_draft.draft_state = DraftState::Draft;
            let mut theirs = open_pr(3, "hubot")?;
            theirs.requested_reviewers = vec![Reviewer::User(me.clone())];
            let mut their_draft = open_pr(4, "hubot")?;
            their_draft.draft_state = DraftState::Draft;

            let value = project(
                vec![
                    mine_draft,
                    open_pr(2, "octocat")?,
                    theirs,
                    their_draft,
                    open_pr(5, "hubot")?,
                ],
                &enforced_rule()?,
                &me,
            );
            Ok::<_, InvalidValue>(node(&value, "/sections").clone())
        });

        assert_eq!(
            sections,
            Ok(json!({
                "mine":    [{"n": 2, "section": "mine"}],
                "mine_drafts": [{"n": 1, "section": "mine_drafts"}],
                "review":  [{"n": 3, "section": "review"}],
                "drafts":  [{"n": 4, "section": "drafts"}],
                "neither": [],
                "unclaimed": [{"n": 5, "section": "unclaimed"}],
            })),
            "the draft sorts last among the viewer's own, as the board ordered it"
        );
    }

    #[test]
    fn a_pull_request_carries_the_counts_and_verdicts_the_diff_compares() {
        let pr = Login::parse("octocat").and_then(|me| {
            let mut busy = open_pr(7, "octocat")?;
            busy.review_decision = Some(ReviewDecision::Approved);
            busy.mergeable = MergeableState::Conflicting;
            busy.merge_state = MergeState::Dirty;
            busy.unresolved_threads = 2;
            busy.comments = 9;
            busy.changes_requested_by = vec![Login::parse("zeta")?, Login::parse("alpha")?];
            busy.checks = vec![
                check("Preview CI Gate", "FAILURE")?,
                check("codecov/patch", "FAILURE")?,
                check("build / test", "IN_PROGRESS")?,
                check("Validate PR Title", "SUCCESS")?,
            ];

            let value = project(vec![busy], &enforced_rule()?, &me);
            Ok::<_, InvalidValue>(node(&value, "/prs/0").clone())
        });

        assert_eq!(
            pr,
            Ok(json!({
                "n": 7,
                "author": "octocat",
                "draft": false,
                "governed": true,
                "fail_blocking": 1,
                "fail_advisory": 1,
                "running": 1,
                "total": 4,
                "unresolved_threads": 2,
                "comments": 9,
                "mergeable": "CONFLICTING",
                "merge_state": "DIRTY",
                "review_decision": "APPROVED",
                "requested": [],
                "unnamed": 0,
                "changes_requested_by": ["alpha", "zeta"],
                "blocked": "changes requested by zeta, alpha",
            })),
            "changes_requested_by is sorted for the diff; the sentence keeps the reported order"
        );
    }

    #[test]
    fn an_author_whose_account_is_gone_is_null_rather_than_a_name() {
        // Null rather than the literal string `unknown`: a deleted account has
        // no login, and a consumer must not be able to mistake the absence for
        // somebody actually called that.
        let author = Login::parse("octocat").and_then(|me| {
            let mut orphaned = open_pr(1, "hubot")?;
            orphaned.author = None;
            orphaned.requested_reviewers = vec![Reviewer::User(me.clone())];
            let value = project(vec![orphaned], &enforced_rule()?, &me);
            Ok::<_, InvalidValue>(node(&value, "/prs/0/author").clone())
        });

        assert_eq!(author, Ok(Value::Null));
    }

    #[test]
    fn the_api_enums_go_back_out_spelled_the_way_github_spelled_them() {
        // The domain renames two of GitHub's values -- `UNKNOWN` becomes
        // `Computing` on one enum and `Undetermined` on the other, because a
        // catch-all called `Unknown` already exists. The wire spelling has to
        // come back for the diff, or every row with a mergeability still being
        // computed reads as a difference.
        let spellings = Login::parse("octocat").and_then(|me| {
            let mut computing = open_pr(1, "octocat")?;
            computing.mergeable = MergeableState::Computing;
            computing.merge_state = MergeState::Undetermined;
            computing.review_decision = Some(ReviewDecision::parse("SOMETHING_NEW"));

            let mut unseen = open_pr(2, "octocat")?;
            unseen.mergeable = MergeableState::parse("SOMETHING_NEW");
            unseen.merge_state = MergeState::parse("ODD_STATE");
            unseen.review_decision = Some(ReviewDecision::ChangesRequested);

            let value = project(vec![computing, unseen], &enforced_rule()?, &me);
            Ok::<_, InvalidValue>(
                (0..2)
                    .map(|index| {
                        json!([
                            node(&value, &format!("/prs/{index}/mergeable")),
                            node(&value, &format!("/prs/{index}/merge_state")),
                            node(&value, &format!("/prs/{index}/review_decision")),
                        ])
                    })
                    .collect::<Vec<_>>(),
            )
        });

        assert_eq!(
            spellings,
            Ok(vec![
                json!(["UNKNOWN", "UNKNOWN", "SOMETHING_NEW"]),
                json!(["SOMETHING_NEW", "ODD_STATE", "CHANGES_REQUESTED"]),
            ])
        );
    }

    #[test]
    fn requested_reviewers_are_sorted_by_name_and_the_nameless_are_counted() {
        // `requested` is sorted so the diff does not turn on the order GitHub
        // returned. `unnamed` exists because the sorted list cannot hold a
        // reviewer with no name, and dropping one made the sentence report
        // fewer people than were asked.
        let asked = Login::parse("octocat").and_then(|me| {
            let mut waiting = open_pr(1, "octocat")?;
            waiting.requested_reviewers = vec![
                Reviewer::User(Login::parse("zeta")?),
                Reviewer::unknown("Organization"),
                Reviewer::Team(TeamSlug::new("ledger-owners")),
                Reviewer::User(Login::parse("alpha")?),
                Reviewer::unknown("Organization"),
            ];
            let value = project(vec![waiting], &enforced_rule()?, &me);
            Ok::<_, InvalidValue>(json!([
                node(&value, "/prs/0/requested"),
                node(&value, "/prs/0/unnamed"),
            ]))
        });

        assert_eq!(asked, Ok(json!([["alpha", "ledger-owners", "zeta"], 2])));
    }

    #[test]
    fn an_ungoverned_pull_request_says_so() {
        // `governed` cannot be read off a row: a row under no rule and a row
        // under a rule with nothing advisory look identical once classified.
        // It is reported from the rule the row was judged by.
        let governance = Login::parse("octocat").and_then(|me| {
            let governed = project(vec![open_pr(1, "octocat")?], &enforced_rule()?, &me);
            let ungoverned = project(
                vec![open_pr(1, "octocat")?],
                &PolicyRules::unprotected(),
                &me,
            );
            Ok::<_, InvalidValue>(json!([
                node(&governed, "/prs/0/governed"),
                node(&ungoverned, "/prs/0/governed"),
            ]))
        });

        assert_eq!(governance, Ok(json!([true, false])));
    }

    #[test]
    fn the_policy_object_reports_the_rule_the_board_was_judged_against() {
        let policy = enforced_rule().and_then(|rules| {
            let me = Login::parse("octocat")?;
            let value = project(vec![open_pr(1, "octocat")?], &rules, &me);
            Ok::<_, InvalidValue>(node(&value, "/policy").clone())
        });

        assert_eq!(
            policy,
            Ok(json!({
                "default_branch": "main",
                "required_checks": ["Preview CI Gate", "Validate PR Title"],
                "have_required": true,
                "required_approvals": 1,
                "conversation_resolution": true,
            }))
        );
    }

    #[test]
    fn an_unprotected_branch_reports_no_required_checks_rather_than_omitting_them() {
        // The dangerous reading is "the key is missing, so who knows". An
        // empty list beside `have_required: false` says the rule was read and
        // named nothing, which is what makes every failure blocking below.
        let policy = Login::parse("octocat").and_then(|me| {
            let value = project(
                vec![open_pr(1, "octocat")?],
                &PolicyRules::unprotected(),
                &me,
            );
            Ok::<_, InvalidValue>(node(&value, "/policy").clone())
        });

        assert_eq!(
            policy,
            Ok(json!({
                "default_branch": "main",
                "required_checks": [],
                "have_required": false,
                "required_approvals": 0,
                "conversation_resolution": false,
            }))
        );
    }

    #[test]
    fn the_document_carries_exactly_the_keys_it_promises() {
        // The key set is a contract with whatever reads `--format json`, so
        // adding or dropping one is a breaking change to a script nobody here
        // can see. Pinned as a list rather than derived, so a change has to be
        // typed on purpose.
        const PR_KEYS: [&str; 17] = [
            "author",
            "blocked",
            "changes_requested_by",
            "comments",
            "draft",
            "fail_advisory",
            "fail_blocking",
            "governed",
            "merge_state",
            "mergeable",
            "n",
            "requested",
            "review_decision",
            "running",
            "total",
            "unnamed",
            "unresolved_threads",
        ];
        const ORACLE_TOP_KEYS: [&str; 3] = ["policy", "prs", "sections"];
        const ORACLE_POLICY_KEYS: [&str; 5] = [
            "conversation_resolution",
            "default_branch",
            "have_required",
            "required_approvals",
            "required_checks",
        ];
        const SECTION_KEYS: [&str; 6] = [
            "drafts",
            "mine",
            "mine_drafts",
            "neither",
            "review",
            "unclaimed",
        ];

        fn keys(value: &Value) -> Vec<String> {
            value
                .as_object()
                .map(|object| object.keys().cloned().collect())
                .unwrap_or_default()
        }

        let document = Login::parse("octocat").and_then(|me| {
            Ok(project(
                vec![open_pr(1, "octocat")?],
                &enforced_rule()?,
                &me,
            ))
        });

        let at = |pointer: &str| document.as_ref().map(|value| keys(node(value, pointer)));

        assert_eq!(at(""), Ok(ORACLE_TOP_KEYS.map(str::to_owned).to_vec()));
        assert_eq!(
            at("/policy"),
            Ok(ORACLE_POLICY_KEYS.map(str::to_owned).to_vec())
        );
        assert_eq!(
            at("/sections"),
            Ok(SECTION_KEYS.map(str::to_owned).to_vec())
        );
        assert_eq!(at("/prs/0"), Ok(PR_KEYS.map(str::to_owned).to_vec()));
    }

    /// One row per rung of the ladder, so a property can be checked across the
    /// whole space rather than at a few points.
    ///
    /// Returns the rows already reduced, each beside the rule that judged it,
    /// because `governed` asks about the rule and a reduced row does not keep
    /// it.
    fn one_row_per_rung() -> Result<Vec<(BoardRow, PolicyRules)>, InvalidValue> {
        let strict = PolicyRules {
            up_to_date_branch: Requirement::Required,
            ..enforced_rule()?
        };
        let approved = Some(ReviewDecision::Approved);
        let green = vec![check("Validate PR Title", "SUCCESS")?];

        let mut rows = Vec::new();
        let mut push = |number: u32, spoil: &dyn Fn(&mut PrFacts)| -> Result<(), InvalidValue> {
            let mut facts = open_pr(number, "octocat")?;
            spoil(&mut facts);
            rows.push((BoardRow::reduce(facts, &strict), strict.clone()));
            Ok(())
        };

        push(1, &|f| {
            f.review_decision = approved.clone();
            f.checks.clone_from(&green);
        })?;
        push(2, &|f| {
            f.review_decision = approved.clone();
            f.merge_state = MergeState::Behind;
            f.checks.clone_from(&green);
        })?;
        push(3, &|f| {
            f.review_decision = approved.clone();
            f.mergeable = MergeableState::Conflicting;
            f.checks.clone_from(&green);
        })?;
        push(4, &|f| {
            f.mergeable = MergeableState::Conflicting;
            f.checks.clone_from(&green);
        })?;
        let blocker = Login::parse("hubot")?;
        push(5, &|f| {
            f.changes_requested_by = vec![blocker.clone()];
            f.checks.clone_from(&green);
        })?;
        let failing = check("Preview CI Gate", "FAILURE")?;
        push(6, &|f| f.checks = vec![failing.clone()])?;
        push(7, &|f| {
            f.unresolved_threads = 2;
            f.checks.clone_from(&green);
        })?;
        let building = check("Preview CI Gate", "IN_PROGRESS")?;
        push(8, &|f| f.checks = vec![building.clone()])?;
        push(9, &|f| {
            f.mergeable = MergeableState::Computing;
            f.checks.clone_from(&green);
        })?;
        let wanted = Login::parse("hubot")?;
        push(10, &|f| {
            f.requested_reviewers = vec![Reviewer::User(wanted.clone())];
            f.checks.clone_from(&green);
        })?;
        push(11, &|f| f.checks.clone_from(&green))?; // nobody assigned
        push(12, &|f| f.draft_state = DraftState::Draft)?;
        push(13, &|_| ())?; // nothing has reported at all

        Ok(rows)
    }

    #[test]
    fn the_rung_fixture_really_does_cover_the_rungs() {
        // The two ordering properties below are quantified over this fixture,
        // so they are only as strong as its coverage: a fixture where every
        // row landed on one rung would satisfy both of them and prove nothing.
        // Counted rather than eyeballed.
        let covered = one_row_per_rung().map(|rows| {
            let mut urgencies: Vec<_> = rows.iter().map(|(row, _)| row.urgency()).collect();
            urgencies.sort_unstable();
            urgencies.dedup();
            let mut reviewabilities: Vec<_> =
                rows.iter().map(|(row, _)| row.reviewability()).collect();
            reviewabilities.sort_unstable();
            reviewabilities.dedup();
            (urgencies.len(), reviewabilities.len())
        });

        assert_eq!(
            covered,
            Ok((11, 7)),
            "every rung of both ladders must appear, or the properties are quantified over nothing"
        );
    }
}
