# Reviewed and stacked pull requests Implementation Plan

> **For Agents:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Draw the pull requests the viewer already reviewed, draw stacked pull requests as one tree in the section of the stack's root, and print no section that has no rows.

**Architecture:** `prtrack-core` owns the grouping. Each board group becomes a `Stacks` (a forest of `Stack` trees) instead of a flat `Vec<BoardRow>`. A stack goes in the section `Section::of` gives its root, and every row remembers its own section, so the summary counts don't move. The CLI renderer walks the trees in pre-order and indents stacked rows. JSON gains a `stacked_on` field.

**Tech Stack:** Rust 2021 workspace (`prtrack-core`, `prtrack-cli`, `prtrack-github`), `serde_json`, clippy with `indexing_slicing` and `wildcard_enum_match_arm` denied through `-D warnings`.

Design: [`2026-10-11-reviewed-and-stacked-prs-design.md`](2026-10-11-reviewed-and-stacked-prs-design.md).

**Never use a real account, organisation, repository, pull request number or branch name** in code, tests, fixtures, docs or commit messages. Use `octocat`, `hubot`, `monalisa`, `example-org/example-repo`, PR numbers like `101`–`105`, and branch names like `stack/one`.

---

## Investigation evidence

**Current path**
- `crates/prtrack-core/src/board.rs`:
  - `enum Section` (191-218) and `Section::of` (228-255). Step 4 returns `NotWaitingOnYou` when `facts.reviewed_by.contains(viewer)`.
  - `struct Board` (361-369) has six `Vec<BoardRow>` fields.
  - `Board::assemble` (378-441) groups the rows, then sorts each group.
  - The six `&[BoardRow]` accessors are at 453-511, and `total` at 518-526.
- `crates/prtrack-cli/src/render.rs`:
  - `BoardText::render` (95-137) draws four sections. Its comment at 107-111 says reviewed rows are deliberately hidden.
  - `section` (186-208) prints a heading, then rows or a placeholder.
  - `row_line(&BoardRow)` (250-307).
  - `summary_line` (326-369) counts `not_waiting_on_you().len()`.
- `crates/prtrack-cli/src/json.rs`: `sections()` (105-124) emits `"neither"` from `not_waiting_on_you()`. `pull_request()` (128-192) writes the 17 `PR_KEYS`.
- Other call sites:
  - `crates/prtrack-cli/src/main.rs:703,711,715` use `total()`, `yours().is_empty()` and `needs_your_review().is_empty()`.
  - `crates/prtrack-github/src/facts.rs:1052-1062` sums group `.len()`s.

**Reuse inventory**
- `Section::of`: the only placement rule, reused per root and per row.
- `PrFacts::head_ref` / `base_ref` (`facts.rs:819,830`): `BranchName` derives `Eq + Hash`, so the head-to-PR index is a `HashMap<&BranchName, Vec<usize>>`. No new GitHub field and no new query.
- The existing sort keys in `assemble`, applied to roots.
- Test builders reused unchanged: `open_pr`, `board_for`, `numbers` (`board.rs` tests); `open_pr`, `render`, `plain` (`render.rs` tests); `project` (`json.rs` tests). Each fixture sets `head_ref`/`base_ref` on a built `PrFacts`, since the fields are public.

**Duplication decision**
- `Stacks` replaces `Vec<BoardRow>` as each group's carrier rather than sitting beside it.
- `Placed` is the one pre-order view the renderer, JSON and summary all read. No parallel depth or parent maps.
- `Stacks::len`/`is_empty` count pull requests, so every existing `.len()`/`.is_empty()` call keeps its meaning.

**Bounded context:** grouping and ordering are board rules, so they live in `prtrack-core`. The renderer only draws.

**Package boundaries:** `prtrack-core`'s public API changes:
- `Board::not_waiting_on_you()` is renamed to `you_reviewed()`.
- Accessors return `&Stacks`.
- New public types: `Stacks`, `Stack`, `Placed`.

Every consumer is in this workspace: `prtrack-cli` and one `prtrack-github` test.

**Scale contract:** one pass to index heads (O(n)), a cycle check bounded at n steps per row (O(n²), with n at most a few hundred open pull requests), and one pre-order walk per draw. No extra requests.

**Rejected alternatives**
- Regrouping rows in the renderer: the ordering would live in the shell and every future caller would have to repeat it.
- A separate parent map passed to the renderer: one concept with two owners.
- A `#[deprecated]` alias for `not_waiting_on_you`: the project forbids keeping dead code that has no callers.

---

### Task 1: Rename `NotWaitingOnYou` to `YouReviewed`

A pure rename; the existing tests are the check. Fix the stale doc comments the investigation found on the lines it touches.

**Files:**
- Modify: `crates/prtrack-core/src/board.rs` (enum variant, field, accessor, doc comments at 189-190, 341-358, 500-517, test comment ~1035)
- Modify: `crates/prtrack-cli/src/render.rs:335,366`
- Modify: `crates/prtrack-cli/src/json.rs:122`
- Modify: `crates/prtrack-github/src/facts.rs:1060`

**Step 1:** Rename `Section::NotWaitingOnYou` → `Section::YouReviewed`, the field `not_waiting_on_you` → `you_reviewed`, and `pub fn not_waiting_on_you` → `pub fn you_reviewed`. Rewrite the accessor's doc to:

```rust
    /// Somebody else's pull requests the viewer has already approved or asked
    /// changes on, ordered by number.
    ///
    /// GitHub removes a reviewer from the request list the moment they submit,
    /// so without checking who has reviewed, work the viewer approved would be
    /// indistinguishable from work nobody has touched.
```

Change "four groups" to "six groups" (lines 189-190, 341, 517). In the `Board` doc (356-358), say a team-only request lands in `Board::unclaimed`. That is what the test at 1125-1166 checks.

**Step 2:** Update the callers in `render.rs`, `json.rs` and `github/facts.rs` to `you_reviewed()`. Keep the JSON key `"neither"`: it is part of `SECTION_KEYS`, which is a contract.

**Step 3:** Run `cargo test --locked --workspace`. Expected: PASS, with the same counts as before.

**Step 4:** Commit.

```bash
git add crates/prtrack-core/src/board.rs crates/prtrack-cli/src/render.rs crates/prtrack-cli/src/json.rs crates/prtrack-github/src/facts.rs
git commit -m "refactor(core): name the reviewed group for what it holds"
```

---

### Task 2: Groups become forests (`Stacks`), every row still its own root

No stack links yet. This task changes the carrier and keeps every output identical.

**Files:**
- Modify: `crates/prtrack-core/src/board.rs`
- Modify: `crates/prtrack-core/src/lib.rs` (re-export `Stack`, `Stacks`, `Placed` beside `Board`)
- Modify: `crates/prtrack-cli/src/render.rs`, `crates/prtrack-cli/src/json.rs`

**Step 1: Write the failing test** (`board.rs` tests):

```rust
    #[test]
    fn a_pull_request_on_nothing_is_a_root_at_depth_zero() {
        let placed = Login::parse("octocat").and_then(|me| {
            let board = board_for(vec![open_pr(101, "hubot")?], &me);
            Ok::<_, InvalidValue>(
                board
                    .walk()
                    .map(|at| (at.row().facts().number.get(), at.depth(), at.stacked_on()))
                    .collect::<Vec<_>>(),
            )
        });
        assert_eq!(placed, Ok(vec![(101, 0, None)]));
    }
```

**Step 2:** Run `cargo test --locked -p prtrack-core a_pull_request_on_nothing`. Expected: FAIL to compile (`walk` not found).

**Step 3: Implement.** Add to `board.rs`:

```rust
/// One pull request, and every pull request stacked on top of it.
///
/// A tree rather than a list: two branches can be cut from the same feature
/// branch, and each becomes a pull request whose base is that branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stack {
    row: BoardRow,
    /// Where this row would sit on its own. A stacked row is drawn in its
    /// root's section, and the counts under the board still read this.
    own: Section,
    above: Vec<Stack>,
}

impl Stack {
    /// The pull request at the bottom of this stack.
    #[must_use]
    pub const fn row(&self) -> &BoardRow {
        &self.row
    }

    /// The pull requests whose base is this one's branch, by number.
    #[must_use]
    pub fn above(&self) -> &[Stack] {
        &self.above
    }

    fn len(&self) -> usize {
        1 + self.above.iter().map(Self::len).sum::<usize>()
    }

    fn walk_into<'a>(&'a self, position: Position, out: &mut Vec<Placed<'a>>) {
        out.push(Placed { row: &self.row, own: self.own, position });
        let depth = position.depth().saturating_add(1);
        for stack in &self.above {
            stack.walk_into(
                Position::Stacked { on: self.row.facts.number, depth },
                out,
            );
        }
    }
}

/// Where a row sits in its tree.
///
/// An enum so that "stacked on nothing at depth three" cannot be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Position {
    Root,
    Stacked { on: PrNumber, depth: usize },
}

impl Position {
    const fn depth(self) -> usize {
        match self {
            Self::Root => 0,
            Self::Stacked { depth, .. } => depth,
        }
    }
}

/// One row as a reader meets it: in pre-order, with how far up a stack it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed<'a> {
    row: &'a BoardRow,
    own: Section,
    position: Position,
}

impl<'a> Placed<'a> {
    /// The pull request.
    #[must_use]
    pub const fn row(&self) -> &'a BoardRow {
        self.row
    }

    /// How many pull requests it is stacked on. Zero for a root.
    #[must_use]
    pub const fn depth(&self) -> usize {
        self.position.depth()
    }

    /// The pull request directly beneath it, or `None` for a root.
    #[must_use]
    pub const fn stacked_on(&self) -> Option<PrNumber> {
        match self.position {
            Position::Root => None,
            Position::Stacked { on, .. } => Some(on),
        }
    }
}

/// One section of the board: its stacks, roots in the section's order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Stacks(Vec<Stack>);

impl Stacks {
    /// The trees, roots in order.
    #[must_use]
    pub fn stacks(&self) -> &[Stack] {
        &self.0
    }

    /// How many pull requests, counting every row in every tree.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.iter().map(Stack::len).sum()
    }

    /// Whether the section has no pull requests at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Every row in pre-order: each root, then what is stacked on it.
    #[must_use]
    pub fn walk(&self) -> Vec<Placed<'_>> {
        let mut out = Vec::with_capacity(self.len());
        for stack in &self.0 {
            stack.walk_into(Position::Root, &mut out);
        }
        out
    }

    /// Every row in pre-order, without its position.
    pub fn rows(&self) -> impl Iterator<Item = &BoardRow> {
        self.walk().into_iter().map(|placed| placed.row)
    }
}
```

Change `Section`'s derive to include `Copy` if it doesn't already (it does: `Clone, Copy, PartialEq, Eq`). Import `PrNumber` from `crate::types`.

In `Board`:
- Every field becomes `Stacks`, and `#[derive(... Default)]` is added.
- Every accessor returns `&Stacks`.
- `total` becomes `self.walk().count()`.
- Add:

```rust
    /// Every row on the board, section by section in drawing order.
    pub fn walk(&self) -> impl Iterator<Item = Placed<'_>> {
        [
            &self.needs_your_review,
            &self.unclaimed,
            &self.you_reviewed,
            &self.yours,
            &self.your_drafts,
            &self.other_drafts,
        ]
        .into_iter()
        .flat_map(Stacks::walk)
    }

    /// Rows that would be in the review section on their own: asked of the
    /// viewer, or asked of nobody. Counted by row, not by tree, so a row
    /// stacked into another section still counts where it belongs.
    pub fn awaiting_review(&self) -> impl Iterator<Item = &BoardRow> {
        self.walk()
            .filter(|placed| matches!(placed.own, Section::NeedsYourReview | Section::Unclaimed))
            .map(|placed| placed.row)
    }

    /// How many rows the viewer has already reviewed, wherever they are drawn.
    #[must_use]
    pub fn you_have_reviewed(&self) -> usize {
        self.walk()
            .filter(|placed| placed.own == Section::YouReviewed)
            .count()
    }
```

Rewrite `assemble` so each row becomes a single-node `Stack { row, own, above: Vec::new() }` pushed into `self.group_mut(own)`. Add:

```rust
    fn group_mut(&mut self, section: Section) -> &mut Stacks {
        match section {
            Section::Yours => &mut self.yours,
            Section::YourDrafts => &mut self.your_drafts,
            Section::NeedsYourReview => &mut self.needs_your_review,
            Section::Unclaimed => &mut self.unclaimed,
            Section::OtherDrafts => &mut self.other_drafts,
            Section::YouReviewed => &mut self.you_reviewed,
        }
    }
```

Move the existing sort keys onto `stack.row` (for example `self.yours.0.sort_by_key(|stack| (stack.row.urgency(), stack.row.facts.last_push, stack.row.facts.number.get()))`). Keep each comment with its sort, and delete the duplicated comment block at 420-425.

Update the `board.rs` test helper:

```rust
    fn numbers(group: &Stacks) -> Vec<u32> {
        group.rows().map(|row| row.facts().number.get()).collect()
    }
```

**Step 4: Callers.**

`render.rs`:
- `section(&self, title: &str, rows: &[Placed<'_>], when_empty: &str, with_columns: bool)`. The heading counts `rows.len()`, and the row line becomes `self.row_line(placed.row())`.
- The review section becomes `self.board.needs_your_review().walk().into_iter().chain(self.board.unclaimed().walk()).collect::<Vec<_>>()`. Other sections use `self.board.x().walk()`.
- In `summary_line`, `every_row` becomes `self.board.walk().map(|placed| placed.row())`.
- `reviewable` becomes `self.board.awaiting_review().filter(|row| row.reviewability() < Reviewability::CiRunning).count()`.
- The reviewed count becomes `self.board.you_have_reviewed()`.

`json.rs`: `listed(rows: &Stacks, section: &str)` iterates `rows.rows()`.

**Step 5:** Run `cargo test --locked --workspace`. Expected: PASS, including the new test.

**Step 6:** Commit.

```bash
git add crates/prtrack-core/src/board.rs crates/prtrack-core/src/lib.rs crates/prtrack-cli/src/render.rs crates/prtrack-cli/src/json.rs
git commit -m "refactor(core): hold each board group as a forest of stacks"
```

---

### Task 3: Link stacked pull requests, and place a stack by its root

**Files:**
- Modify: `crates/prtrack-core/src/board.rs`

**Step 1: Write the failing tests.** Add a fixture builder next to `open_pr`:

```rust
    /// `open_pr`, on `head` and aimed at `base`.
    fn on_branch(number: u32, author: &str, head: &str, base: &str) -> Result<PrFacts, InvalidValue> {
        let mut pr = open_pr(number, author)?;
        pr.head_ref = BranchName::new(head);
        pr.base_ref = BranchName::new(base);
        Ok(pr)
    }

    /// Every row as (number, depth, stacked on), in drawing order.
    fn shape(group: &Stacks) -> Vec<(u32, usize, Option<u32>)> {
        group
            .walk()
            .into_iter()
            .map(|at| (at.row().facts().number.get(), at.depth(), at.stacked_on().map(PrNumber::get)))
            .collect()
    }
```

Tests (table-driven where three or more cases share a shape):

```rust
    #[test]
    fn a_stack_is_one_tree_in_order_from_the_bottom() {
        // Handed in top first, so the order is the board's work, not the input's.
        let drawn = Login::parse("octocat").and_then(|me| {
            let board = board_for(
                vec![
                    on_branch(103, "hubot", "stack/three", "stack/two")?,
                    on_branch(101, "hubot", "stack/one", "main")?,
                    on_branch(102, "hubot", "stack/two", "stack/one")?,
                ],
                &me,
            );
            Ok::<_, InvalidValue>(shape(board.unclaimed()))
        });
        assert_eq!(
            drawn,
            Ok(vec![(101, 0, None), (102, 1, Some(101)), (103, 2, Some(102))])
        );
    }

    #[test]
    fn a_base_that_cannot_be_named_with_certainty_makes_a_root() {
        // A merged base, two open pull requests sharing a head, and a loop: in
        // each the honest answer is "this sits on nothing we can show".
        let cases: [(&str, Vec<(u32, &str, &str)>); 3] = [
            ("base already merged", vec![(102, "stack/two", "stack/one")]),
            (
                "two pull requests share the base's name",
                vec![
                    (101, "stack/one", "main"),
                    (104, "stack/one", "main"),
                    (102, "stack/two", "stack/one"),
                ],
            ),
            (
                "a loop",
                vec![(101, "stack/one", "stack/two"), (102, "stack/two", "stack/one")],
            ),
        ];
        for (what, prs) in cases {
            let roots = Login::parse("octocat").and_then(|me| {
                let built = prs
                    .iter()
                    .map(|&(n, head, base)| on_branch(n, "hubot", head, base))
                    .collect::<Result<Vec<_>, _>>()?;
                let board = board_for(built, &me);
                Ok::<_, InvalidValue>(
                    shape(board.unclaimed()).iter().all(|&(_, depth, on)| depth == 0 && on.is_none()),
                )
            });
            assert_eq!(roots, Ok(true), "{what}: every row must be a root");
        }
    }

    #[test]
    fn a_stack_is_drawn_in_its_roots_section_and_counted_where_each_row_belongs() {
        // The bottom was reviewed by the viewer; the two above it were not.
        let placed = Login::parse("octocat").and_then(|me| {
            let mut bottom = on_branch(101, "hubot", "stack/one", "main")?;
            bottom.reviewed_by = vec![me.clone()];
            let board = board_for(
                vec![
                    bottom,
                    on_branch(102, "hubot", "stack/two", "stack/one")?,
                    on_branch(103, "hubot", "stack/three", "stack/two")?,
                ],
                &me,
            );
            Ok::<_, InvalidValue>((
                numbers(board.you_reviewed()),
                numbers(board.unclaimed()),
                board.you_have_reviewed(),
                board.awaiting_review().count(),
                board.total(),
            ))
        });
        assert_eq!(placed, Ok((vec![101, 102, 103], vec![], 1, 2, 3)));
    }

    #[test]
    fn a_draft_at_the_bottom_takes_its_stack_with_it() {
        let placed = Login::parse("octocat").and_then(|me| {
            let mut bottom = on_branch(101, "hubot", "stack/one", "main")?;
            bottom.draft_state = DraftState::Draft;
            let board = board_for(
                vec![bottom, on_branch(102, "hubot", "stack/two", "stack/one")?],
                &me,
            );
            Ok::<_, InvalidValue>((numbers(board.other_drafts()), numbers(board.unclaimed())))
        });
        assert_eq!(placed, Ok((vec![101, 102], vec![])));
    }

    #[test]
    fn two_branches_cut_from_one_are_siblings_by_number() {
        let drawn = Login::parse("octocat").and_then(|me| {
            let board = board_for(
                vec![
                    on_branch(105, "hubot", "stack/b", "stack/one")?,
                    on_branch(101, "hubot", "stack/one", "main")?,
                    on_branch(103, "hubot", "stack/a", "stack/one")?,
                ],
                &me,
            );
            Ok::<_, InvalidValue>(shape(board.unclaimed()))
        });
        assert_eq!(
            drawn,
            Ok(vec![(101, 0, None), (103, 1, Some(101)), (105, 1, Some(101))])
        );
    }
```

**Step 2:** Run `cargo test --locked -p prtrack-core stack`. Expected: the first, third, fourth and fifth tests FAIL, because everything is still a root. The second test passes; that is correct, since it pins the fallback.

**Step 3: Implement.** Add (imports: `std::collections::HashMap`, `crate::facts::BranchName`):

```rust
/// The pull request each row is stacked on, by index, or `None` for a root.
///
/// A row sits on the one open pull request whose head is its base. None, more
/// than one, or itself all make it a root: guessing between two branches that
/// share a name would draw a stack that does not exist. Any row whose chain of
/// bases loops back to itself is a root too, so building the trees ends.
fn parents(rows: &[BoardRow]) -> Vec<Option<usize>> {
    let mut by_head: HashMap<&BranchName, Vec<usize>> = HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        by_head.entry(&row.facts.head_ref).or_default().push(index);
    }
    let mut parents: Vec<Option<usize>> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| match by_head.get(&row.facts.base_ref).map(Vec::as_slice) {
            Some(&[only]) if only != index => Some(only),
            Some(_) | None => None,
        })
        .collect();

    let looping: Vec<bool> = (0..parents.len())
        .map(|start| {
            let mut at = parents.get(start).copied().flatten();
            for _ in 0..parents.len() {
                match at {
                    Some(next) if next == start => return true,
                    Some(next) => at = parents.get(next).copied().flatten(),
                    None => return false,
                }
            }
            false
        })
        .collect();
    for (parent, looped) in parents.iter_mut().zip(looping) {
        if looped {
            *parent = None;
        }
    }
    parents
}

/// The tree rooted at `at`, taking each row out of `slots` as it is placed.
fn grow(
    at: usize,
    slots: &mut [Option<BoardRow>],
    children: &[Vec<usize>],
    viewer: &Login,
) -> Option<Stack> {
    let row = slots.get_mut(at)?.take()?;
    let mut above: Vec<Stack> = children
        .get(at)
        .into_iter()
        .flatten()
        .filter_map(|&child| grow(child, slots, children, viewer))
        .collect();
    above.sort_by_key(|stack| stack.row.facts.number.get());
    Some(Stack {
        own: Section::of(&row.facts, viewer),
        row,
        above,
    })
}
```

`assemble` becomes:

```rust
    pub fn assemble(rows: Vec<BoardRow>, viewer: &Login) -> Self {
        let parents = parents(&rows);
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); rows.len()];
        for (child, parent) in parents.iter().enumerate() {
            if let Some(below) = parent.and_then(|parent| children.get_mut(parent)) {
                below.push(child);
            }
        }
        let mut slots: Vec<Option<BoardRow>> = rows.into_iter().map(Some).collect();

        let mut board = Self::default();
        for (root, parent) in parents.iter().enumerate() {
            if parent.is_some() {
                continue;
            }
            if let Some(stack) = grow(root, &mut slots, &children, viewer) {
                board.group_mut(stack.own).0.push(stack);
            }
        }
        board.sort();
        board
    }
```

Move the sorting from Task 2 into `fn sort(&mut self)`. Every row is reachable from a root because `parents` leaves no loop, and `total()` (via `walk`) is the guard: the existing partition test `every_pull_request_lands_in_exactly_one_section_and_none_is_dropped` fails if a row goes missing.

Update the `Board` and `you_reviewed` docs: a group now holds every stack *rooted* there.

**Step 4:** Run `cargo test --locked --workspace`. Expected: PASS.

**Step 5: Mutation check.** Make `parents` return `vec![None; rows.len()]` and confirm the stack tests fail. Remove the `only != index` guard and the loop cut in turn, and confirm `a_base_that_cannot_be_named_with_certainty_makes_a_root` fails for each (or the run hangs on the loop, which is also a failure). Restore the file.

**Step 6:** Commit.

```bash
git add crates/prtrack-core/src/board.rs
git commit -m "feat(core): a stacked pull request is drawn on the one beneath it"
```

---

### Task 4: Draw a stack as an indented tree, columns still aligned

**Files:**
- Modify: `crates/prtrack-cli/src/render.rs`

**Step 1: Write the failing test** (render tests; add `BranchName` to the test module's `prtrack_core` import if it's missing):

```rust
    #[test]
    fn a_stack_is_drawn_under_the_pull_request_it_sits_on() {
        let rendered = Login::parse("octocat").and_then(|me| {
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
                plain(),
            ))
        })
        .unwrap_or_default();
        let lines: Vec<&str> = rendered.lines().collect();
        let line_of = |needle: &str| lines.iter().position(|line| line.contains(needle));

        assert!(lines.iter().any(|line| line.starts_with("#101 ")), "{rendered}");
        assert!(lines.iter().any(|line| line.starts_with("└ #102 ")), "{rendered}");
        assert!(lines.iter().any(|line| line.starts_with("  └ #103 ")), "{rendered}");
        assert!(
            line_of("#101 ") < line_of("#102 ") && line_of("#102 ") < line_of("#103 "),
            "{rendered}"
        );
        // Every row's author column starts where the header's does.
        let author_at = |line: &str| line.find("hubot").or_else(|| line.find("monalisa"));
        let header = lines.iter().find(|line| line.starts_with("PR ")).copied().unwrap_or_default();
        for n in ["#101 ", "#102 ", "#103 ", "#104 "] {
            let row = line_of(n).and_then(|at| lines.get(at)).copied().unwrap_or_default();
            assert_eq!(author_at(row), header.find("AUTHOR"), "{n} misaligned:\n{rendered}");
        }
    }
```

**Step 2:** Run `cargo test --locked -p prtrack-cli a_stack_is_drawn`. Expected: FAIL (no `└`).

**Step 3: Implement.**
- Add `const TREE_STEP: usize = 2;` with the column constants, documented as the width each level of stacking adds to the PR column.
- Add:

```rust
    /// How much wider the PR column is than [`PR_COLUMN`], so that the deepest
    /// stack on the board still leaves every other column aligned.
    fn tree_width(&self) -> usize {
        self.board.walk().map(|placed| placed.depth()).max().unwrap_or(0) * TREE_STEP
    }
```

- `column_header` pads `"PR"` to `PR_COLUMN + self.tree_width()`.
- `row_line` takes `placed: &Placed<'_>`, with `let row = placed.row();`. The PR cell becomes:

```rust
            pad_to(
                &format!(
                    "{}{}",
                    tree_prefix(placed.depth()),
                    palette.link(
                        facts.url.as_str(),
                        &palette.paint(Style::Plain, &format!("#{}", facts.number.get())),
                    )
                ),
                PR_COLUMN + self.tree_width()
            ),
```

with

```rust
/// The indent and branch mark in front of a stacked row's number.
fn tree_prefix(depth: usize) -> String {
    match depth.checked_sub(1) {
        None => String::new(),
        Some(levels_below) => format!("{}└ ", " ".repeat(levels_below * TREE_STEP)),
    }
}
```

- `section` passes each `Placed` to `row_line`.

**Step 4:** Run `cargo test --locked -p prtrack-cli`. Expected: PASS, and every older layout test unchanged, because a board with no stacks has `tree_width() == 0`.

**Step 5:** Commit.

```bash
git add crates/prtrack-cli/src/render.rs
git commit -m "feat(cli): draw a stack as a tree under the pull request it sits on"
```

---

### Task 5: Draw `YOU REVIEWED`

**Files:**
- Modify: `crates/prtrack-cli/src/render.rs`

**Step 1: Write the failing test:**

```rust
    #[test]
    fn a_pull_request_you_reviewed_stays_on_the_board_with_its_blocker() {
        let rendered = Login::parse("octocat").and_then(|me| {
            let mut reviewed = open_pr(101, "hubot")?;
            reviewed.reviewed_by = vec![me.clone()];
            reviewed.mergeable = MergeableState::Conflicting;
            Ok(render(vec![reviewed], &PolicyRules::unprotected(), &me, plain()))
        })
        .unwrap_or_default();
        assert!(rendered.contains("YOU REVIEWED  (1)"), "{rendered}");
        let row = rendered.lines().find(|line| line.starts_with("#101 ")).unwrap_or_default();
        assert!(row.contains("CONFLICT"), "{rendered}");
    }
```

Use the `MergeableState` path the render tests already import. If `mergeable` isn't how the existing conflict tests set it, copy their setup.

**Step 2:** Run it. Expected: FAIL (no heading, no row).

**Step 3:** In `render`, draw `YOU REVIEWED` from `self.board.you_reviewed().walk()` right after NEED YOUR REVIEW, with columns. Replace the comment at 107-111 with:

```rust
        // What you have already reviewed has a section of its own rather than
        // a line in the review list: asking you to review your own approval is
        // the fastest way to teach somebody to stop reading that list, and
        // hiding it lost the pull requests that went bad after you approved.
```

**Step 4:** Run `cargo test --locked -p prtrack-cli`. Expected: PASS.

**Step 5:** Commit.

```bash
git add crates/prtrack-cli/src/render.rs
git commit -m "fix(cli): a pull request you reviewed stays on the board"
```

---

### Task 6: Print no section that has no rows

**Files:**
- Modify: `crates/prtrack-cli/src/render.rs`

**Step 1: Replace the test** `an_empty_section_says_it_is_empty_rather_than_vanishing` (1535-1558) with:

```rust
    #[test]
    fn a_section_with_nothing_in_it_is_not_printed() {
        let rendered = Login::parse("octocat")
            .and_then(|me| Ok(render(vec![open_pr(101, "hubot")?], &PolicyRules::unprotected(), &me, plain())))
            .unwrap_or_default();
        assert!(rendered.contains("NEED YOUR REVIEW  (1)"), "{rendered}");
        for absent in ["YOUR PRS", "YOUR DRAFTS", "OTHER DRAFTS", "YOU REVIEWED", "nothing open", "  none"] {
            assert!(!rendered.contains(absent), "{absent:?} printed:\n{rendered}");
        }
        assert!(!rendered.contains("\n\n\n"), "no run of blank lines where a section was:\n{rendered}");
        // The summary still tells an empty board from a failed read: the
        // failure exits non-zero and never reaches here.
        assert!(rendered.contains("1 open"), "{rendered}");
    }
```

**Step 2:** Run it. Expected: FAIL (`YOUR PRS  (0)` is printed).

**Step 3: Implement.** `section` loses `when_empty` and returns an empty `Vec` for no rows. `render` builds the section blocks, drops empty ones, and joins the rest with one blank line:

```rust
        let sections = [
            self.section("NEED YOUR REVIEW", &review, true),
            self.section("YOU REVIEWED", &self.board.you_reviewed().walk(), true),
            self.section("YOUR PRS", &self.board.yours().walk(), true),
            self.section("YOUR DRAFTS", &self.board.your_drafts().walk(), true),
            self.section("OTHER DRAFTS", &self.board.other_drafts().walk(), false),
        ];
        for block in sections.into_iter().filter(|block| !block.is_empty()) {
            lines.extend(block);
            lines.push(String::new());
        }
```

Keep the existing per-section comments next to their lines.

**Step 4:** Run `cargo test --locked -p prtrack-cli`. Expected: PASS. `a_section_heading_counts_the_rows_beneath_it` is unaffected.

**Step 5:** Commit.

```bash
git add crates/prtrack-cli/src/render.rs
git commit -m "feat(cli): print no section that has nothing in it"
```

---

### Task 7: `stacked_on` in JSON

**Files:**
- Modify: `crates/prtrack-cli/src/json.rs`

**Step 1: Write the failing test:**

```rust
    #[test]
    fn a_stacked_pull_request_names_the_one_beneath_it() {
        let document = Login::parse("octocat").and_then(|me| {
            let mut bottom = open_pr(101, "hubot")?;
            bottom.head_ref = BranchName::new("stack/one");
            let mut top = open_pr(102, "hubot")?;
            top.head_ref = BranchName::new("stack/two");
            top.base_ref = BranchName::new("stack/one");
            Ok(project(vec![top, bottom], &PolicyRules::unprotected(), &me))
        })
        .unwrap_or_default();
        let stacked_on = |n: u64| {
            document["prs"]
                .as_array()
                .and_then(|prs| prs.iter().find(|pr| pr["n"] == n))
                .map(|pr| pr["stacked_on"].clone())
        };
        assert_eq!(stacked_on(101), Some(Value::Null));
        assert_eq!(stacked_on(102), Some(json!(101)));
    }
```

Add `"stacked_on"` to `PR_KEYS` in `the_document_carries_exactly_the_keys_it_promises`, making the array 18 long.

**Step 2:** Run it. Expected: FAIL.

**Step 3: Implement.** In `to_value`, build `let stacked_on: HashMap<u32, u32> = self.board.walk().filter_map(|at| Some((at.row().facts().number.get(), at.stacked_on()?.get()))).collect();` and pass `stacked_on.get(&number).copied()` into `pull_request` as a new `stacked_on: Option<u32>` parameter, inserting `fields.insert("stacked_on".to_owned(), json!(stacked_on));`.

**Step 4:** Run `cargo test --locked -p prtrack-cli`. Expected: PASS.

**Step 5:** Commit.

```bash
git add crates/prtrack-cli/src/json.rs
git commit -m "feat(cli): JSON names the pull request a stacked one sits on"
```

---

### Task 8: README, version, verification

**Files:**
- Modify: `README.md:9-23` (example board)
- Modify: `crates/prtrack-cli/Cargo.toml` (version `0.1.8`), `Cargo.lock` (`cargo update -p prtrack-cli --offline`)

**Step 1:** Replace the README example with a board in the new shape. It uses `octocat`, `hubot` and `monalisa`, PR numbers 101-106, `stack/*` branches, a three-level stack, a YOU REVIEWED section, and no empty sections. Generate it rather than typing it: build a release binary and run it against a throwaway repository you own, or hand-assemble the output from the tests' `render()` with the same rows. Check it with `git grep -niE "<any real org or login you know of>"`. That must return nothing.

**Step 2:** Bump `prtrack-cli` to `0.1.8` so merging cuts the release.

**Step 3:** Run `/code-review`, then `/verify`. The smoke run is `prtrack board` against a repository with a stack. Confirm the tree renders, the columns align, no empty section prints, and `--format json | jq '.prs[] | select(.stacked_on != null)'` lists the stacked rows.

**Step 4:** Commit and open the PR.
