# Reviewed and stacked pull requests on the board

## Problem

Two failures, both found on a live repository with a three-PR stack:

1. **A pull request disappears once the viewer reviews it.** `Section::of` sends
   anything the viewer has approved or requested changes on to
   `Board::not_waiting_on_you`, which is counted ("9 you have reviewed") but
   never drawn. A reviewed pull request that later conflicts is invisible.
2. **Stacks are invisible.** In a stack `main ← A ← B ← C`, the board listed B
   and C as clean and reviewable, and hid A, which conflicted with `main` and
   carried the viewer's change request. The rows were each correct and the
   picture they made was wrong.

## Decisions

- **Reviewed pull requests get their own section, `YOU REVIEWED`**, under the
  review section. Each row keeps its real blocker and MERGE column, so a
  conflict shows. The review section stays a to-do list.
- **A stack is drawn as one tree inside one section.** The bottom unmerged pull
  request (the root) decides the section, and the whole stack goes there, so a
  stack is never split across sections.
- **Each pull request keeps its own reason.** A pull request stacked on a
  blocked one is not relabelled "waiting on #N". The blocked one sits directly
  above it in the tree, and reviewing an upper diff is still useful work, so it
  still counts as reviewable and can still nudge.
- **Empty sections are not printed.**

## Design

### Core (`prtrack-core`, `board.rs`)

- `Section::NotWaitingOnYou` becomes `Section::YouReviewed`, and
  `Board::not_waiting_on_you()` becomes `Board::you_reviewed()`. This is a rename
  of a public API with no callers outside this workspace yet, so no deprecated
  alias is kept.
- **Parent link.** A pull request's parent is the open pull request whose
  `head_ref` equals its `base_ref`. It is a root when:
  - no open pull request has that head (the base is `main`, or a merged branch);
  - **more than one** does (two forks pushing the same branch name), because
    guessing between them would draw a stack that does not exist;
  - following parents would loop, which breaks the cycle into roots rather than
    letting assembly run forever.
- **Section.** A stack takes the section `Section::of` gives its root. Roots
  are ordered by the section's existing keys. The pull requests stacked on a
  node are ordered by number.
- **Nothing is dropped.** `Board::total()` still counts every pull request, and
  every pull request appears exactly once in exactly one tree.

### Pull requests from forks (decided during implementation)

Linking by branch name alone mis-links onto a pull request opened from a fork:
a fork's `main`, aimed at `main` here, would have every pull request based on
`main` drawn on top of it. Review caught this during implementation, and the
user chose to read GitHub's `isCrossRepository`:

- `PrFacts::head_repository: HeadRepository { ThisOne, Fork, Unstated }`,
  converted from the wire's `isCrossRepository` (`false`, `true`, absent).
- Only a `ThisOne` head can have something stacked on it. A fork can still sit
  on a branch here, since its base is in this repository.
- `Unstated` is treated as "not here": a missing edge is better than a wrong
  one. The recorded fixtures predate the field, so every recorded pull request
  is `Unstated`. They are not re-recorded, because the recorder keeps real pull
  request numbers. The drift test lists the field in
  `ASKED_FOR_AFTER_THE_RECORDING` until the next re-recording.

### Renderer (`prtrack-cli`, `render.rs`, `json.rs`)

- A stacked row is indented under its parent with `└`, one step per level. Its
  CI, MERGE and reason columns are its own.
- `YOU REVIEWED` is rendered after the review section.
- A section with no rows prints nothing at all: no heading, no column header,
  no placeholder.
- JSON gains an additive `stacked_on` field: the parent's number, or `null`.

### Unchanged

The per-pull-request reason ladder, `--watch` transition rules and nudges, and
the summary-line counts.

## Testing

Core first, each test seen failing before its code exists:

- parent link: a plain stack, a base with no open pull request, an ambiguous
  head, and a cycle;
- a stack follows its root's section, including a root that is a draft or one
  the viewer reviewed;
- totals equal the input length with stacks present.

Renderer: indentation for a three-level stack, `YOU REVIEWED` drawn, an empty
section printing nothing, and `stacked_on` in JSON. Fixtures use `octocat`-style
names and `example-org/example-repo`.
