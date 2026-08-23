---
name: verify
description: Run the full verification suite — format, clippy, build, tests, docs, and a real CLI smoke run. This is the definition of done before pushing.
allowed-tools: Bash, Read, Grep, Glob
---

# /verify — Full Verification Suite

> Run `/code-review` before `/verify`. Both skills source
> `.agents/skills/code-review/lib/base-ref.sh` so consecutive runs after a push
> only re-check the new diff window.

Run the complete verification pipeline that must pass before pushing code.

## Step 0: Pick a base ref and classify the changes

This step gates everything else. Skip the heavy pipeline when the change set
does not need it, and reuse the same effective base ref throughout.

Before choosing, identify the last commit covered by a successful local
verification or pushed checkpoint, and decide whether this is the initial branch
check or a repeat. Only the initial check may cover the whole branch. A repeat
must cover only the unverified increment.

**0a. Pick the effective base ref**

```bash
source .agents/skills/code-review/lib/base-ref.sh
echo "Verify base ref: $BASE_REF"
```

The `github-stacked-pr` skill sets `CR_BASE_REF_OVERRIDE=origin/main` when it
verifies the cumulative state from the top layer. The helper requires any
override to exist and be an ancestor of `HEAD`. Do not set or guess the override
without first validating native stack topology.

Set `VERIFY_DIFF_BASE` to the base of the coverage actually needed:

- Initial whole-branch check: `VERIFY_DIFF_BASE="$BASE_REF"`.
- Repeat from a verified pushed branch tip: use that remote branch tip.
- Repeat after one non-merge commit whose parent is verified:
  `VERIFY_DIFF_BASE="HEAD^"`.
- Repeat immediately after merging `origin/main` when the first parent is
  verified: `VERIFY_DIFF_BASE="HEAD^1"`.

The first-parent merge diff includes the newly integrated upstream files. That
is intentional: it exercises them against the combined branch tree without
repeating the already-proven PR range.

If none of those bases exactly describes the unverified increment, stop and
report the gap. Do not widen to a convenient fallback.

**0b. Classify the changed files**

```bash
git diff --name-only "$VERIFY_DIFF_BASE...HEAD"
```

Bucket each path:

| Bucket | What's in it |
|--------|--------------|
| **docs-only** | `*.md`, `docs/**`, `AGENTS.md`, `CLAUDE.md`, `.agents/**`, `.claude/**`, `.codex/**` |
| **workflow-only** | `.github/workflows/**`, `.github/actions/**`, issue/PR templates, label configs |
| **code/mixed** | `crates/**`, `src/**`, `tests/**`, `benches/**`, `examples/**`, `Cargo.toml`, `Cargo.lock`, `build.rs`, `rust-toolchain.toml`, `.cargo/**`, `Dockerfile*`, OR any mix that includes one of these |

**0c. Route to the matching gate**

- **docs-only** — check links and code fences in the changed markdown, then jump
  to Step 6. Do not build or run tests.
- **workflow-only** — run `actionlint` (when installed), then jump to Step 6.
- **code/mixed** — proceed through Steps 1–6.

If you skip the heavy pipeline, say so explicitly in the Step 6 report so the
user can see *why* verify finished in seconds.

## Step 1: Format and lint

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

`cargo fmt --check` must be run before clippy: a formatting failure is cheap to
detect and makes the clippy output diff-noisy.

If either fails, stop and report. Do NOT proceed to build.

When `clippy` reports a warning in a file this branch touched, fix it — do not
add an `#[allow]`. An inline allow hides the warning from every later count.

## Step 2: Build

```bash
cargo build --workspace --all-targets --locked
```

`--locked` is required. A build that silently updates `Cargo.lock` is not the
build CI will run.

If it fails, stop and report. Do NOT proceed to tests.

## Step 3: Tests

Choose the narrowest command that covers the unverified increment.

| Situation | Command |
|-----------|---------|
| Initial whole-branch check, or a change to a shared crate | `cargo test --workspace --all-features --locked` |
| Repeat check confined to one crate | `cargo test -p <crate> --all-features --locked` |
| Public API or docs changed | add `cargo test --doc --workspace` |

Run the narrow command first. If it passes and the diff touched only that
crate's own files, that is sufficient for a repeat check. If the diff touched a
crate that others depend on, the workspace run is required regardless of how
small the change looks.

If tests fail, stop and report. Do NOT proceed.

## Step 4: Docs and public surface

Run when the diff touched any `pub` item, `lib.rs`, or doc comments:

```bash
cargo doc --workspace --no-deps --all-features
```

Treat a `rustdoc` warning about a broken intra-doc link as a failure — it means
a doc comment references something that no longer exists.

If the diff changed a public item, also confirm the version bump matches the
change class (breaking → major, additive → minor). `cargo-semver-checks` proves
this mechanically when installed:

```bash
command -v cargo-semver-checks >/dev/null && cargo semver-checks check-release
```

Report `SKIPPED — cargo-semver-checks not installed` rather than silently
omitting the line.

## Step 5: CLI smoke run

A green test suite does not prove the binary works. Tests exercise functions;
they do not prove the terminal is restored, the table renders, or auth resolves.

```bash
cargo run --locked -- --help
cargo run --locked -- --version
```

Then run the real command the change affects, against a real repo, and read the
output:

- Confirm the table renders and columns align.
- Confirm the process exits cleanly on Ctrl-C and the terminal is restored
  (no leftover raw mode, cursor visible, prompt normal).
- Confirm piped output is clean: `cargo run --locked -- <cmd> | cat` must
  contain no ANSI escapes.
- If the change touched auth or the API client, run once with no token
  available and confirm the error is a readable message on stderr with a
  non-zero, non-1 exit code — not a panic.

If the change cannot be smoke-tested without credentials the session does not
have, say so explicitly in the Step 6 report rather than marking it PASS.

## Step 6: Report results

Always state the base ref, and call out any step skipped by Step 0c routing.

For a docs-only or workflow-only run:

```
Verification Results (docs-only, base=origin/<branch>):
  Docs check: PASS
```

For a code/mixed run:

```
Verification Results (base=origin/<branch>):
  fmt:        PASS/FAIL
  clippy:     PASS/FAIL
  build:      PASS/FAIL
  tests:      PASS/FAIL  (<command actually run>)
  docs:       PASS/FAIL/SKIPPED
  semver:     PASS/FAIL/SKIPPED
  smoke run:  PASS/FAIL/SKIPPED (<what was actually exercised>)
```

Never report PASS for a step you did not run. "SKIPPED" with a reason is a
correct report; an unearned PASS is not.

If everything passed, report ready to push. If anything failed, report what
needs fixing.
