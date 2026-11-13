# macOS Menubar Spike Implementation Plan

> **For Agents:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Prove, end to end, that a SwiftUI `MenuBarExtra` app can show the
board computed by `prtrack-core`, fetched by `prtrack-github`, across a UniFFI
boundary, using only the Command Line Tools (no Xcode).

**Architecture:** A new `prtrack-ffi` crate is the only crate that knows UniFFI
exists. It exports one async function, `fetch_board(repo)`, which runs the
existing GitHub client on a tokio runtime, reduces and assembles the board with
the existing core functions, and projects the result into the two sections the
popover shows. A SwiftPM package under `apps/macos/` links the static library
and renders it. Swift owns the wording and Rust owns every judgement.

**Tech Stack:** Rust 1.95, `uniffi` 0.32 (async exports on tokio), Swift 6.2
via SwiftPM, SwiftUI `MenuBarExtra` with the `.window` style, macOS 14+.

**Decisions already made (2026-10-21):**
1. The boundary lives in a new `prtrack-ffi` crate. Core takes no uniffi dependency.
2. `unsafe_code = "forbid"` moves from `[workspace.lints]` into each crate root,
   so the FFI crate (whose generated glue is `unsafe`) can still inherit
   every other workspace lint.
3. Rust fetches. Swift never talks to GitHub.

**What the spike must answer.** Write the answers into the PR description:
- Does a UniFFI async export on tokio behave from a SwiftUI `Task`, including
  cancellation when the popover closes?
- Can the whole thing be built with the Command Line Tools alone? This machine
  has no Xcode: `xcodebuild` fails with "requires Xcode".
- Which workspace lints, if any, does the UniFFI-generated scaffolding trip?
- What do the static library and the app binary weigh?

**Out of scope:** notifications (`UNUserNotificationCenter` needs a signed
bundle), Keychain token storage, polling and `--watch` transition state,
signing and notarisation, universal builds, an Xcode project, and any change
to the CLI's behaviour.

---

## Investigation evidence

> **Re-verified 2026-11-12 against `origin/main` at `b5c1da2`.** Six PRs landed
> after the first draft. #10 made sections trees (`&Stacks`), and #11 added
> `you_reviewed` and a viewer-adjusted reason on `Placed`. Line numbers below
> are from `b5c1da2`.

**Current path.** The CLI composes the board in two production places, and the
code is identical in both:
- `crates/prtrack-cli/src/main.rs:230-236` (inside `watch`) and
  `crates/prtrack-cli/src/main.rs:517-526` (`board`) each do
  `facts.into_parts()`, map every `(PrFacts, PolicyRules)` through
  `BoardRow::reduce`, and call `Board::assemble(rows, &viewer)`. The test
  `crates/prtrack-github/src/facts.rs:1197-1205` repeats it a third time.
  (`watch.rs:271`, `json.rs:343` and `render.rs:771` are test helpers that judge
  under a single rule. They are a different shape and stay as they are.)
- Fetching is `GitHubClient::from_env()` → `fetch_open_pull_requests(&repo)` →
  `RepositoryFacts::try_from(fetched)`. See `fetch` and `read_repository` in `main.rs`.
- Sections come from `Board::needs_your_review()` and `Board::yours()`
  (`crates/prtrack-core/src/board.rs:679,698`). Each returns `&Stacks`, a list of
  trees of stacked pull requests, not a row slice. `Stacks::walk()` (`:526`)
  yields `Placed` rows in pre-order with `depth()`. `Board::total()` is `:792`.
- **The reason to show is `Placed::reason()` (`board.rs:483`), not
  `BoardRow::reason()`.** On a re-review, `Placed::reason` sets aside the
  viewer's own answered change request, so the two differ exactly when the row
  is in front of the viewer *because* they are due to re-review.
- A row's facts are public fields on `PrFacts` (`number`, `url`, `title`,
  `author`), reached through `Placed::row().facts()`.

**Reuse inventory.**
| Need | Existing | Classification |
| --- | --- | --- |
| Fetch with pagination and retry | `GitHubClient::{from_env, fetch_open_pull_requests}` | reuse |
| Facts out of the wire shape | `RepositoryFacts::try_from`, `into_parts` | reuse |
| Reduce and assemble | `BoardRow::reduce`, `Board::assemble` | **extract**: the FFI's would be the third production copy |
| Section order, stacks, viewer-adjusted reason | `Stacks::walk`, `Placed::{row, depth, reason}` | reuse |
| Which rung, as a flat enum | `prtrack_core::ReasonKind` and `ReasonKind::of` (`nudge.rs:54-90`) | reuse via `#[uniffi::remote(Enum)]` |
| Offline test data | `crates/prtrack-github/tests/fixtures/open-pull-requests-page-{1,2}.json`, `wiremock`, `GitHubClient::with_endpoint` | reuse |
| Reason wording | `prtrack_cli::json::plain_reason` (`json.rs:244`) | **not reused**: it lives in a shell; Swift is a separate shell and owns its own wording |

**Duplication decision.**
- `ReasonKind` already exists as the flat, payload-free form of
  `BlockedReason`. The FFI re-exports it with `#[uniffi::remote(Enum)]`
  instead of declaring a lookalike. UniFFI checks that the remote declaration
  matches, so a new rung breaks the build rather than drifting.
- `PopoverBoard` and `PopoverRow` are new. They are not a parallel domain
  carrier: they are the projection one shell needs, in the same way that
  `json.rs` builds a `serde_json::Value` for `--format json`. They hold strings
  and numbers only and make no decisions.
- `FetchError` is new and has a single variant carrying the error's `Display`
  chain. The CLI's classification (`prtrack_cli::exit::Exit::of`) sits in a
  shell and cannot be reached from here. Mapping it to typed Swift errors is
  the real app's job, not the spike's.

**Bounded context.** Every judgement (rung, section, ordering) stays in
`prtrack-core`. Fetching and policy resolution stay in `prtrack-github`.
`prtrack-ffi` is a shell, like `prtrack-cli`: it may depend on both crates, and
nothing may depend on it.

**Package boundaries.** The dependency direction is `prtrack-ffi → prtrack-github →
prtrack-core`, the same as `prtrack-cli`. One public contract is added:
`RepositoryFacts::judge` (Task 2) in `prtrack-github`.

**Scale contract.** One fetch per refresh, which is the same GraphQL cost as
`prtrack board`. The projection is O(rows) with one `String` clone per field,
and rows number in the tens. Nothing is cached on the Rust side.

**Rejected alternatives.**
- *UniFFI derives inside `prtrack-core`.* Rejected by decision 1.
- *A Swift `enum Reason` mirroring `ReasonKind` by hand.* That would be a
  parallel type with no compile-time check. The remote enum gives Swift a
  generated, exhaustive enum instead.
- *Depending on `prtrack-cli` for `plain_reason`.* That would make one shell
  depend on another. `AGENTS.md` says nothing in that library is reusable
  outside its binary.
- *An `.xcframework`.* Building one needs `xcodebuild -create-xcframework`,
  which needs Xcode. The spike links the static library directly.

---

## Task 0: Branch

The current checkout is `fix/paginate-check-contexts`, which is unrelated.

```bash
git fetch origin
git worktree add .claude/worktrees/macos-spike -b spike/macos-menubar origin/main
```

Work in that worktree from here on.

## Task 1: Move the unsafe forbid into each crate root

**Files:**
- Modify: `Cargo.toml` (`[workspace.lints.rust]`: delete `unsafe_code = "forbid"`)
- Modify: `crates/prtrack-github/src/lib.rs` (add `#![forbid(unsafe_code)]` beside the other inner attributes)
- Modify: `crates/prtrack-cli/src/main.rs` (add `#![forbid(unsafe_code)]`; it is a separate crate root from `lib.rs`)
- Already present: `crates/prtrack-core/src/lib.rs:46`, `crates/prtrack-cli/src/lib.rs:13`

**Step 1: Prove that the forbid currently comes from the workspace.**
Add `const _: () = unsafe { };` to the top of `crates/prtrack-github/src/lib.rs`, then run:

```bash
cargo check -p prtrack-github --locked
```

Expected: an error mentioning `unsafe_code`.

**Step 2:** Delete the workspace entry and add the attribute to the two crate
roots. Replace the deleted line with a comment explaining that the forbid now
sits in each crate root, because `prtrack-ffi` carries generated `unsafe` glue
and Cargo cannot override a workspace lint per crate.

**Step 3:** Run `cargo check -p prtrack-github --locked`. Expected: still an
error, now raised by the crate attribute. Remove the probe and check again:
expected to pass. Repeat the probe in `crates/prtrack-cli/src/main.rs`.

**Step 4: Commit.**
```bash
git add Cargo.toml crates/prtrack-github/src/lib.rs crates/prtrack-cli/src/main.rs
git commit -m "build: forbid unsafe per crate root, not workspace-wide"
```

## Task 2: Extract the reduce-and-assemble step

**Files:**
- Modify: `crates/prtrack-github/src/facts.rs` (add `RepositoryFacts::judge` beside `into_parts`, `:85-100`)
- Modify: `crates/prtrack-github/src/lib.rs` (export `Judged`)
- Modify: `crates/prtrack-cli/src/main.rs:230-236`, `:517-526`
- Modify: `crates/prtrack-github/src/facts.rs:1197-1205` (the existing test that composes by hand)

**Step 1: Write the failing test.** Put it in the `#[cfg(test)] mod tests` of
`facts.rs`. Build the `RepositoryFacts` with the same helper the existing tests
there use. Assert that `judge()` returns one row per pair, that each row equals
`BoardRow::reduce(facts, &policy)` for its pair, and that the board equals
`Board::assemble` over those rows. Use a fixture with **at least two pull
requests governed by different rules**. A single-rule fixture cannot
distinguish "each row judged by its own rule" from "every row judged by the
default rule".

**Step 2:** Run `cargo test -p prtrack-github --locked judge`. Expected: it
fails to compile because `judge` does not exist yet.

**Step 3: Implement.**

```rust
/// A repository's pull requests, each reduced under the rule that governs it,
/// and the board they assemble into.
///
/// Each row keeps its rule beside it because `--format json` reports
/// `governed`, and that cannot be recovered from a classified row.
#[derive(Debug)]
pub struct Judged {
    /// Whose board this is.
    pub viewer: Login,
    /// The repository's default branch, where it has one.
    pub default_branch: Option<BranchName>,
    /// The rule protecting the default branch.
    pub default_rule: PolicyRules,
    /// Every row with the rule that judged it.
    pub rows: Vec<(BoardRow, PolicyRules)>,
    /// The rows in sections.
    pub board: Board,
}

impl RepositoryFacts {
    /// Reduces every pull request under its own rule and assembles the board.
    #[must_use]
    pub fn judge(self) -> Judged {
        let (viewer, default_branch, default_rule, pairs) = self.into_parts();
        let rows: Vec<(BoardRow, PolicyRules)> = pairs
            .into_iter()
            .map(|(facts, policy)| (BoardRow::reduce(facts, &policy), policy))
            .collect();
        let board = Board::assemble(rows.iter().map(|(row, _)| row.clone()).collect(), &viewer);
        Judged { viewer, default_branch, default_rule, rows, board }
    }
}
```

**Step 4:** Migrate both CLI call sites and the `facts.rs:1197` test to
`facts.judge()`. Rename only where the field names force it. Expected diff:
about eight lines removed per site.

**Step 5: Mutation check.** Temporarily change `judge` to pass `&default_rule`
instead of `&policy`. The test must fail. Revert.

**Step 6:** Run `cargo test --workspace --locked`. Expected: everything passes.

**Step 7: Commit.**
```bash
git add crates/prtrack-github/src/facts.rs crates/prtrack-github/src/lib.rs crates/prtrack-cli/src/main.rs
git commit -m "refactor(github): judge a repository's pull requests in one place"
```

## Task 3: The `prtrack-ffi` crate

**Files:**
- Create: `crates/prtrack-ffi/Cargo.toml`
- Create: `crates/prtrack-ffi/src/lib.rs`
- Create: `crates/prtrack-ffi/src/bin/uniffi-bindgen.rs`
- Create: `crates/prtrack-ffi/tests/fetch_board.rs`

**Step 1: Manifest.**

```toml
[package]
name = "prtrack-ffi"
version = "0.0.0"
description = "The UniFFI boundary the macOS app links. A shell: nothing depends on it."
edition.workspace = true
license.workspace = true
publish = false

[lib]
crate-type = ["staticlib", "lib"]

[[bin]]
name = "uniffi-bindgen"
required-features = ["bindgen"]

[features]
bindgen = ["uniffi/cli"]

[dependencies]
prtrack-core = { path = "../prtrack-core" }
prtrack-github = { path = "../prtrack-github" }
tokio = { workspace = true }
uniffi = { version = "0.32", features = ["tokio"] }

[dev-dependencies]
wiremock = { workspace = true }
tokio = { workspace = true }

[lints]
workspace = true
```

Add `uniffi = "0.32"` to `[workspace.dependencies]` instead if a second crate
ever needs it. Until then, YAGNI.

**Step 2: Write the failing test** (`tests/fetch_board.rs`). Serve
`../prtrack-github/tests/fixtures/open-pull-requests-page-1.json` and then
`page-2.json` from `wiremock`, copying how the existing pagination tests in
`crates/prtrack-github/src/wire.rs` sequence the two pages. Point the client at
the mock with `GitHubClient::with_endpoint`. Call
`prtrack_ffi::fetch_board_with(&client, "example-org/example-repo")`. It is a
plain `pub` Rust function, not UniFFI-exported, so the integration test can
reach it. Assert:
- The numbers in `needs_your_review` and in `yours` equal, in order,
  `board.needs_your_review().walk()` and `board.yours().walk()` from a `Board`
  built from the same fixture via `judge()`. Compare against the board, not
  literals, so that re-recording the fixture cannot make the test lie.
- A mock answering 500 yields `Err(FetchError::Failed { .. })`, not an empty board.

**Step 2b: the synthetic re-review test** (a unit test of `project`, in
`src/lib.rs`'s `mod tests`). The recorded repository may not contain a
re-review, and a fixture cannot cover a value its repository never produced
(`AGENTS.md`, Testing). Build `PrFacts` by hand: copy how the CLI's tests do it
(`crates/prtrack-cli/src/watch.rs` tests build `PrFacts` and a `PolicyRules`).
Make it somebody else's pull request where **the viewer's own** changes-requested
review stands on an older head, and the head has moved since, so the row lands
in `needs_your_review` with `Placed::rereview() == Some(ChangesRequested)`.
Assert that the projected `reason` equals `ReasonKind::of(placed.reason())`
**and** differs from `ReasonKind::of(placed.row().reason())`. The second
assertion proves that the fixture discriminates. If you cannot make the two
differ, stop and report it rather than weakening the test.

**Step 3:** Run `cargo test -p prtrack-ffi --locked`. Expected: it fails to compile.

**Step 4: Implement `src/lib.rs`.**

```rust
//! The boundary the macOS app links.
//!
//! A shell, like `prtrack-cli`: it fetches, lets the core judge, and projects
//! the result into the two sections the popover shows. Wording belongs to the
//! Swift side; every decision belongs to the core.
//!
//! No `#![forbid(unsafe_code)]` here, unlike every other crate root: the
//! scaffolding `uniffi::setup_scaffolding!` generates is `unsafe` FFI glue.
//! Nothing hand-written in this crate may use `unsafe`.

use prtrack_core::{Board, Placed, RepoRef, ReasonKind, Stacks};
use prtrack_github::{GitHubClient, RepositoryFacts};

uniffi::setup_scaffolding!();

#[uniffi::remote(Enum)]
pub enum ReasonKind {
    Draft, ChangesRequested, RequiredCheckFailing, UnresolvedThreads,
    ApprovedButConflicting, Conflicting, BehindBase, ApprovedCiRunning,
    ReadyToMerge, CiRunning, MergeabilityNotKnown, WaitingForReview, NobodyAssigned,
}

/// One row of the popover.
#[derive(Debug, uniffi::Record)]
pub struct PopoverRow {
    pub number: u32,
    pub title: String,
    pub author: Option<String>,
    pub url: String,
    /// The viewer-adjusted reason: `Placed::reason`, never `BoardRow::reason`.
    pub reason: ReasonKind,
    /// 0 for a stack's root, 1 for what is stacked on it, and so on.
    pub depth: u32,
}

/// What the popover shows: who you are blocking, then your own work.
#[derive(Debug, uniffi::Record)]
pub struct PopoverBoard {
    pub viewer: String,
    pub needs_your_review: Vec<PopoverRow>,
    pub yours: Vec<PopoverRow>,
    pub total: u32,
}

/// Why there is no board. Never an empty one.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum FetchError {
    #[error("{message}")]
    Failed { message: String },
}

/// Fetches `repo` with the token in `GH_TOKEN` or `GITHUB_TOKEN`.
#[uniffi::export(async_runtime = "tokio")]
pub async fn fetch_board(repo: String) -> Result<PopoverBoard, FetchError> {
    let client = GitHubClient::from_env().map_err(failed)?;
    fetch_board_with(&client, &repo).await
}
```

Then write `fetch_board_with`, which parses the repository with `RepoRef::parse`,
fetches, applies `RepositoryFacts::try_from` and then `.judge()`, and projects.
Also write `project(board: &Board, viewer: &Login)`, `section(s: &Stacks) ->
Vec<PopoverRow>` (which maps `s.walk()` in order) and `row(p: &Placed<'_>)`. The
reason **must** be `ReasonKind::of(p.reason())`. `depth` is `p.depth()`
converted with `u32::try_from`, never `as`. Treat an overflow as a
`FetchError`, not as a silent clamp. `you_reviewed` is deliberately left out of
the popover for the spike.
`failed` formats the error with its `source()` chain joined by `": "`. Add
`thiserror = { workspace = true }` to the manifest. Exact field names and the
`Login`/`PrTitle` accessors are at `facts.rs:756-790` and `types.rs:309,388,482`.
Write doc comments on every public item: `missing_docs` is a workspace lint.

**Step 5: `src/bin/uniffi-bindgen.rs`.**
```rust
//! The binding generator, pinned to this crate's uniffi version.
fn main() {
    uniffi::uniffi_bindgen_main();
}
```

**Step 6:** Run `cargo test -p prtrack-ffi --locked`. Expected: it passes. Then:

```bash
cargo clippy -p prtrack-ffi --all-targets --locked -- -D warnings
```

**STOP POINT:** if the generated scaffolding trips any lint (likely candidates
are `missing_docs`, `unreachable_pub` and `clippy::pedantic`), do **not** add
`#[allow]`. Record which lint fires and on which generated item, then ask the
user. `AGENTS.md` permits a generated-code carve-out only with a comment, and
choosing one is their decision. This finding is one of the spike's answers.

**Step 7: Mutation check.** Swap `needs_your_review` and `yours` in `project`.
The test must fail. Revert. Then make `fetch_board_with` return an empty board
on error. The 500 case must fail. Revert. Then project `p.row().reason()`
instead of `p.reason()`. The re-review test must fail. Revert. Commit **before**
mutation-checking, so that `git checkout --` restores the code rather than
wiping it.

**Step 8: Commit.**
```bash
git add crates/prtrack-ffi Cargo.lock
git commit -m "feat(ffi): export fetch_board over UniFFI for the macOS app"
```

## Task 4: The binding build script

**Files:**
- Create: `scripts/build-macos-spike.sh`
- Modify: `.gitignore` (add `apps/macos/Generated/`)

```bash
#!/usr/bin/env bash
# Builds the static library and generates the Swift bindings the macOS spike
# links. Host architecture only: a universal build is out of the spike's scope.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release --locked -p prtrack-ffi
cargo run --release --locked -p prtrack-ffi --features bindgen --bin uniffi-bindgen -- \
  generate --library target/release/libprtrack_ffi.dylib \
  --language swift --out-dir apps/macos/Generated

mkdir -p apps/macos/Sources/PrtrackFFI apps/macos/Sources/prtrack_ffiFFI/include
cp apps/macos/Generated/prtrack_ffi.swift apps/macos/Sources/PrtrackFFI/
cp apps/macos/Generated/prtrack_ffiFFI.h apps/macos/Sources/prtrack_ffiFFI/include/
cp apps/macos/Generated/prtrack_ffiFFI.modulemap apps/macos/Sources/prtrack_ffiFFI/include/module.modulemap
```

The `--library` mode reads metadata from a dynamic library. If the `.dylib` is
not produced alongside the `.a`, add `"cdylib"` to `crate-type`. Record which
one was needed. Also add both copied-into paths to `.gitignore`: generated
code is never committed.

Run it. Expected: the three generated files exist, and so does
`target/release/libprtrack_ffi.a`. Record the size of the `.a`.

```bash
git add scripts/build-macos-spike.sh .gitignore
git commit -m "build: generate the macOS spike's Swift bindings"
```

## Task 5: The SwiftPM menubar app

`apps/` is a new top-level directory. The reason: a Swift package is neither
a crate (`crates/`) nor a script (`scripts/`), and it is the second shell
`AGENTS.md` anticipates. Say so in the PR.

**Files:**
- Create: `apps/macos/Package.swift`
- Create: `apps/macos/Sources/PrtrackMenuBar/PrtrackMenuBarApp.swift`
- Create: `apps/macos/Sources/PrtrackMenuBar/BoardModel.swift`
- Create: `apps/macos/Sources/PrtrackMenuBar/Wording.swift`

**`Package.swift`:**
```swift
// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "PrtrackMenuBar",
    platforms: [.macOS(.v14)],
    targets: [
        .systemLibrary(name: "prtrack_ffiFFI", path: "Sources/prtrack_ffiFFI"),
        .target(
            name: "PrtrackFFI",
            dependencies: ["prtrack_ffiFFI"],
            swiftSettings: [.swiftLanguageMode(.v5)],
            linkerSettings: [
                .unsafeFlags(["-L../../target/release"]),
                .linkedLibrary("prtrack_ffi"),
            ]
        ),
        .executableTarget(name: "PrtrackMenuBar", dependencies: ["PrtrackFFI"]),
    ]
)
```

Language mode 5 applies to the generated target only. Whether the generated
code compiles under Swift 6 strict concurrency is another of the spike's
answers: try `.v6` once and record the result.

**`BoardModel.swift`:**
```swift
import Foundation
import Observation
import PrtrackFFI

@MainActor @Observable
final class BoardModel {
    enum State {
        case loading
        case loaded(PopoverBoard, at: Date)
        case failed(String, lastGood: PopoverBoard?)
    }

    private(set) var state: State = .loading
    let repo: String

    init(repo: String) { self.repo = repo }

    var badge: Int {
        if case let .loaded(board, _) = state { return board.needsYourReview.count }
        return 0
    }

    func refresh() async {
        do {
            state = .loaded(try await fetchBoard(repo: repo), at: .now)
        } catch {
            let lastGood: PopoverBoard? = if case let .loaded(b, _) = state { b } else { nil }
            state = .failed(String(describing: error), lastGood: lastGood)
        }
    }
}
```

**`Wording.swift`:** one exhaustive `switch` over `ReasonKind` with **no
`default:`**, so a new rung fails to compile here. Base the sentences on the
CLI's `plain_reason` (`crates/prtrack-cli/src/json.rs:244`) so that the two
shells agree. They are kept in sync by hand, which is acceptable for a spike.
Write that limitation down in the file.

**`PrtrackMenuBarApp.swift`:** `@main` with `MenuBarExtra { PopoverView }
label: { Label("\(model.badge)", systemImage: "arrow.triangle.pull") }` and
`.menuBarExtraStyle(.window)`. In `init`, call
`NSApplication.shared.setActivationPolicy(.accessory)` so that no Dock icon
appears; without a bundle there is no `LSUIElement`. Read the repository from
`PRTRACK_REPO`, and when it is missing, show that as an explicit error state,
not an empty popover. Build `PopoverView` with two `Section`s, one per array,
and rows of number, wording and author. Add a `.task { await model.refresh() }`
and a Refresh button. When the model has failed, show the error banner **above
the last good board, labelled with its age**, never a bare empty list: tenet 2.

**Build and run:**
```bash
scripts/build-macos-spike.sh
swift build --package-path apps/macos
GH_TOKEN="$(gh auth token)" PRTRACK_REPO=owner/name swift run --package-path apps/macos
```

Expected: an icon with a count appears in the menubar, and clicking it shows
the popover. Record the app binary's size.

```bash
git add apps/macos/Package.swift apps/macos/Sources/PrtrackMenuBar
git commit -m "feat(macos): menubar spike rendering the board over UniFFI"
```

## Task 6: Smoke against the CLI

On the same repository, the popover's two sections must match:

```bash
prtrack board --repo owner/name --format json
```

Compare `needs_your_review` and `yours` row for row, including the order of
stacked rows. Then run the app with
`GH_TOKEN=bad`. Expected: the failure state with GitHub's refusal, **not** an
empty popover. Then cancel mid-fetch by closing the popover. Expected: no crash
and no stale write after close. Note what actually happened, because this is
the cancellation answer.

**Real identities:** the comparison runs against a real repository, so nothing
from that run goes into the repository, the PR or a commit message. Before
pushing, run the grep from `AGENTS.md`.

## Task 7: Definition of done

1. `/code-review`.
2. `/verify`.
3. Check `.github/workflows/ci.yml`. The macOS gate builds the whole workspace,
   so it now builds `prtrack-ffi`. Confirm that the Linux musl job still builds
   only what it ships. If it builds the workspace, `prtrack-ffi` must compile
   there too, and that is a finding, not something to work around.
   The Swift package is **not** added to CI in the spike.
4. Open a **draft** PR titled `spike: macOS menubar over UniFFI`. List the
   spike's answers at the top, then the follow-ups each one implies.
   Do not bump the version: a spike is not a release.
