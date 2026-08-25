# prtrack

Watches GitHub pull requests and tells you the one thing a pull request list
never does: **why each one cannot merge yet** — and interrupts you when that
answer changes.

```
prtrack 0.1.1 | policy on main: 4 required checks | 1 approval(s) | threads must be resolved

YOUR PRS  (15)
PR     AUTHOR      CI          OPEN.CMT MERGE      WHY IT CANNOT MERGE          SINCE PUSH
#3449  octocat     ! 2 adv     0.19     CONFLICT   approved - REBASE NEEDED     22d
#4429  octocat     ! 1 adv     0.11     CONFLICT   conflicting - rebase first   3h
#4349  octocat     ~ 1/42 run  0.11     -          CI running                   1h
#4445  octocat     ok 53/53    0.5      clean      review: hubot +2             3h

NEED YOUR REVIEW  (9)
#4225  octofix     ! 1 adv     0.2      clean      review: octocat +2           6d
#4323  monalisa    ok 34/34    0.11     clean      review: octocat +2           1h
#4324  monalisa    x 3         0.10     -          required check failing       1h

35 open | 0 ready to merge | 7 conflicting | 5 reviewable now
```

`#3449` has been approved and conflicting for twenty-two days. `#4225` has been
waiting for a review for six. Neither shows up as urgent anywhere in GitHub's
interface, and both are somebody else's work blocked on you.

## Install

Requires a GitHub token in `GH_TOKEN` or `GITHUB_TOKEN`, with read access to the
repositories you watch. There is no `--token` flag, deliberately: an argument is
visible to every process on the machine through `ps` and survives in shell
history.

```bash
# macOS — one universal binary, Apple Silicon and Intel
gh release download --repo strigidata/gh-pr-tracker --pattern '*macos*.tar.gz'
tar -xzf prtrack-*-macos-universal.tar.gz
install -m 755 prtrack "$(brew --prefix)/bin/prtrack"   # or any dir on your PATH

# Linux — statically linked, runs on any glibc
gh release download --repo strigidata/gh-pr-tracker --pattern '*linux*.tar.gz'
tar -xzf prtrack-*-linux-x86_64.tar.gz
install -m 755 prtrack ~/.local/bin/prtrack             # or sudo into /usr/local/bin
```

`/usr/local/bin` is **not** writable without `sudo` on Apple Silicon, which is
why the Homebrew prefix is used above. Any directory on your `PATH` will do.

Verify what you downloaded:

```bash
shasum -a 256 -c prtrack-*-macos-universal.tar.gz.sha256   # sha256sum -c on Linux
```

The macOS binary is **not signed or notarized**. Downloaded with `gh` as above
that does not matter — the API is not LaunchServices, so nothing tags the file.
Downloaded through a **browser** it is quarantined and macOS refuses to run it
with "cannot be opened because the developer cannot be verified", which needs:

```bash
xattr -d com.apple.quarantine prtrack
```

Signing removes the caveat entirely and is the gate to distributing outside the
team, not a nicety. Linux has no equivalent obstacle.

## Use

```bash
prtrack board --repo owner/name              # once
prtrack board --repo owner/name --watch      # keep watching, notify on change
prtrack board --repo owner/name --format json
```

Exit codes are a contract: `0` a board, `1` nothing open, `2` bad command line
or invisible repository, `3` refused token, `4` GitHub or the network failed.
An empty board and a failed fetch print almost the same thing, and only the exit
code says which happened.

## Reading the board

| Cell | Meaning |
| --- | --- |
| `x 2` | two **required** checks failing — this is what blocks the merge |
| `x 1+1` | one required failing, plus one advisory alongside |
| `! 2 adv` | two failing, none required — a real failure, but not the blocker |
| `~ 5/41 run` | still running |
| `ok 28/28` | green |
| `no checks` | nothing ever ran, which is **not** the same as passing |
| `0.19` | unresolved threads . comments; the first digit is red when the repository requires resolution |

`MERGE` stays blank until CI is green, because a merge state whose checks have
not finished is a guess that changes under the reader. A row nobody can act on
is dimmed — except its status cells, which keep their colour, because the dim
says "not yours yet" and the colour says "this is what is wrong".

The pull request number is a link. Your terminal probably supports it.

## Features

- **Policy-driven, not cosmetic.** Blocking is decided by the repository's own
  branch protection rules, so only the checks that can actually stop a merge are
  painted as blockers. On a repository with sixty checks and four required ones,
  painting every failure red misdiagnoses most of the board.
- **Thirteen reasons, one per pull request.** A ladder answers a single
  question — why can this not merge yet — and the highest rung that applies is
  the answer. Waiting on a review outranks being behind the base branch, because
  updating a branch changes not one line of the diff a reviewer would read.
- **`--watch` notifies on transitions, not conditions.** Became ready to merge,
  approved and then conflicted, a required check went red, a review was
  requested of you, an approval dismissed by a push. A pull request that has
  been broken for a week does not notify you again every five minutes.
- **It tells you when it cannot interrupt you.** A macOS Focus mode swallows
  notifications silently and the posting API reports success either way; Linux
  needs a session bus that is absent over SSH. Both are reported at startup, so
  silence is never mistaken for nothing happening.
- **Failure is never disguised as emptiness.** A fetch whose page counts do not
  add up is refused rather than returned short, because a board missing a row
  reads as a repository with less open work than it has and nothing downstream
  can tell.

## Architecture

Three crates, split so the library half knows nothing about terminals. A macOS
app and a hosted backend are meant to consume the same crates over a UniFFI
boundary, so their public API is a contract with callers that do not exist yet.

```
prtrack-core     no IO at all. Newtypes with private fields and smart
                 constructors, so an illegal value cannot be built. The
                 thirteen-rung ladder, the board's sections and ordering,
                 and the transition rules behind --watch. Takes `now` as a
                 parameter; never reads a clock.

prtrack-github   the transport. Every GitHub enum is a real Rust enum with an
                 explicit unknown variant — never a bare String, never a
                 wildcard arm — because GitHub adds values and an unrecognised
                 one that classifies as passing is a false green.

prtrack-cli      the shell. Argument parsing, rendering, colour, exit codes,
                 state file, notifications. Everything that assumes a terminal
                 lives here and nowhere else.
```

Two decisions worth knowing before changing anything:

**Pagination orders on `CREATED_AT`, not `UPDATED_AT`.** A cursor walks a sorted
list, so ordering on a key that changes mid-walk lets a pull request move from
ahead of the cursor to behind it and never be seen. The page totals are checked
against the collected count as well, because a pull request *created* mid-walk
is still missed.

**Checks are merged by recency, not by outcome.** One check name reports many
attempts, and taking the newest per name is the only rule that cannot invent a
winner. Counting every historical attempt as current is how a re-run that passed
still reads as a failure.

## Development

```bash
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

The lint set is `all` + `pedantic` + `nursery` + `cargo`, with the panic family,
`indexing_slicing` and `wildcard_enum_match_arm` on top. `#[allow]` is not used
to silence a warning.

Behaviour is verified by mutation, not by reading. That is what found the
vacuous tests — a probe already in normal form cannot test normalisation, a
recorded fixture cannot cover a value the recorded repository does not produce,
and a guard that cannot fire is worse than no guard because the code then lies
about where the protection is.

See [`AGENTS.md`](AGENTS.md) for the full conventions and
[`docs/plans/`](docs/plans/) for the design record.
