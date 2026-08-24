# prtrack v0 — the nudge agent

> **For Agents:** REQUIRED SUB-SKILL: Use superpowers:executing-plans.
> Load `rust-best-practices` before writing any `.rs` file.

**Supersedes** [`2026-08-23-prtrack-audit.md`](2026-08-23-prtrack-audit.md) as the
active plan. The audit tool is deferred: `scripts/pr-latency.sh` already covers
the internal measurement need, and a distributable audit binary only matters
once there are prospects to hand it to.

**Goal:** A local agent that fires one OS notification when a pull request has
been ready and unreviewed past a threshold, and then shuts up about it.

**Architecture:** A pure rule engine over normalised facts, wrapped by a poll
loop and a notification shell. The engine is developed first against fixtures
and does no IO. Delivery is swappable — OS notification in v0, Slack or a hosted
backend later — because the engine only emits events.

**Tech stack:** Rust 2021, `reqwest` + `serde` (hand-written wire types),
`chrono`, `notify-rust`, `clap`, `thiserror` in libs, `anyhow` in the binary.

**In v0:** the agent, plus `prtrack status` — a plain one-shot render to stdout.
`watch -n 300 prtrack status` is the self-updating dashboard, for free.

**Not in v0:** a real TUI (ratatui, raw mode, alternate screen, resize handling,
terminal restore), menubar app, config files, packaging, code signing, App
Store, GitLab, Slack, hosted anything. Env vars and a state file are enough.

**Why `status` is not optional.** A nudge-only agent that has been silent for
three days is indistinguishable from a dead one — expired token, broken query,
crashed process, all look exactly like "nothing crossed the threshold." Tenet 2
says failure must always be distinguishable from an empty result, and a
background agent with no liveness signal violates it. `status` is also the only
way to tune the threshold without guessing, and the only demo artifact that is
not "trust me, it beeps."

---

## Why this shape

Measured against `example-org/example-repo` (see the
[design doc](2026-08-23-pr-nudge-validation-design.md) §2): settled wait p50
5.3h, p90 40.8h, and **15% of PRs wait over 24h after the author stopped
touching them**. PR size does not predict the wait, so this is an attention
problem, not an effort problem.

That dictates the design. **The agent must stay silent for the 85% that are
fine.** A tool that nudges on every PR reproduces the firehose-channel failure
mode that already demonstrably does not work on this team. One message, once,
per PR that crosses the line.

GitHub cannot express this at any frequency setting: its reminders are a
*digest of everything awaiting you* at clock times. Set that to every 30 minutes
and you get the same list sixteen times a day. The difference is not cadence,
it is **per-PR state**.

## The rule, stated precisely

For each open PR, with `settled = now − last_push`:

| Condition | Event |
|---|---|
| Viewer is a requested reviewer, has not reviewed, `settled > threshold` | `NudgeReviewer` |
| Viewer is the author, **no reviewer requested**, `settled > threshold` | `NudgeAuthorUnassigned` |
| Otherwise | nothing |

**De-duplication key is `(pr_number, event_kind, head_sha)`.** Firing is recorded
against that key and never repeated. Using the head SHA means a push
automatically re-arms the nudge — the author changed something, the clock
restarts, no separate cooldown logic needed. This is the whole state machine.

Unassigned PRs nudge the **author**, not a channel and not a round-robin pick.
An unassigned PR is an author-side omission; the tool should say so rather than
make staffing decisions.

## Crates

```
crates/
  prtrack-core/     facts → settled wait → rule → NudgeEvent   (pure, no IO)
  prtrack-github/   open-PR query, wire types, TryFrom
  prtrack-agent/    poll loop · state file · notification delivery
```

`prtrack-core` does no IO and takes `now` as a parameter. Untestable-by-clock is
the most common way a rule engine like this rots.

**Note:** this queries **open** PRs. The audit script queries merged ones. The
fields differ — `headRefOid` and `reviewRequests` matter here, `mergedAt` and
`mergedBy` do not.

---

## Task 1: De-risk the notification path FIRST

The only unknown that could blow the one-day estimate. Do it before anything
else; if it fails, the whole delivery choice changes.

A bare binary has no bundle identifier and macOS **silently drops** the
notification ([notify-rust#132](https://github.com/hoodie/notify-rust/issues/132)).

**Step 1:** New workspace, `crates/prtrack-agent` with a `main.rs` that sends
one notification via `notify-rust`, calling
`set_application(&get_bundle_identifier_or_default("Terminal"))` on macOS first.

**Step 2:** `cargo run -p prtrack-agent`. A notification must actually appear.

**Step 3:** Do not discard the `Result` — unwrapping the error surfaces the
missing-bundle case directly. If nothing appears, check System Settings →
Notifications for Terminal, and confirm Focus is off.

**Step 4:** Commit `chore: verify macos notification delivery path`.

Ship a real `.app` bundle later; note that TCC permission is keyed to bundle ID
plus signature, so a stable ad-hoc signature (`codesign -s -`) beats none.

## Task 2: Workspace, lints, domain newtypes

Reuse Tasks 1–2 of the audit plan verbatim: workspace `Cargo.toml` with the
strict clippy standard repeated per crate, then `PrNumber`, `Login`, `RepoRef`
with smart constructors and private fields. TDD, same five steps.

Add `HeadSha` as a newtype here — it is the de-duplication key and must not be
interchangeable with any other string.

## Task 3: Open-PR fact types

```rust
pub struct OpenPrFacts {
    pub number: PrNumber,
    pub head: HeadSha,
    pub author: Author,                     // Human(Login) | Bot(Login)
    pub ready_at: Option<DateTime<Utc>>,
    pub last_push: DateTime<Utc>,
    pub requested_reviewers: Vec<Login>,
    pub human_reviews: Vec<ReviewFact>,     // bot reviews already filtered out
}
```

`Author` is an enum so bot filtering is a `match`, not a string-suffix check at
call sites. Bot reviews are filtered at the boundary — AI reviewers respond in
~4 minutes and would mask the entire signal.

TDD the `ReviewState` enum with an explicit `Unknown(String)` variant, per
`AGENTS.md`.

## Task 4: Settled wait (pure)

`settled_wait(facts, now) -> Duration`, clamped at zero.

Table cases: never pushed since ready; pushed after ready; `last_push` in the
future (clock skew — clamp, do not go negative); still a draft (no wait, drafts
are not waiting on anyone).

## Task 5: The rule engine — the heart of the product

`evaluate(facts, viewer, config, now, already_fired) -> Option<NudgeEvent>`

Table cases, and these are the specification:

- reviewer requested, no review, over threshold → `NudgeReviewer`
- reviewer requested, **has** reviewed → nothing
- under threshold → nothing
- already fired for this `(pr, kind, head_sha)` → nothing
- already fired, then **head_sha changed** → fires again
- author, no reviewers requested, over threshold → `NudgeAuthorUnassigned`
- author, reviewers requested → nothing (not the author's problem)
- draft → nothing
- bot-authored PR → nothing

Nine cases. If any is wrong the product is either silent or spam.

## Task 6: GitHub open-PR query

```graphql
query($owner:String!, $name:String!) {
  rateLimit { cost remaining }
  repository(owner:$owner, name:$name) {
    pullRequests(first: 50, states: OPEN, orderBy: {field: UPDATED_AT, direction: DESC}) {
      nodes {
        number isDraft headRefOid
        author { __typename login }
        reviewRequests(first: 20) { nodes { requestedReviewer { ... on User { login } } } }
        reviews(first: 20) { nodes { submittedAt state author { __typename login } } }
        timelineItems(last: 10, itemTypes: [PULL_REQUEST_COMMIT, READY_FOR_REVIEW_EVENT]) {
          nodes { __typename
            ... on PullRequestCommit { commit { committedDate } }
            ... on ReadyForReviewEvent { createdAt } } }
      }
    }
  }
}
```

Wire types are `pub(crate)` and converted once with `TryFrom`. Test against a
recorded fixture; **no network in tests**.

Measured cost is ~1 point per page — the budget is a non-issue. Retry with
backoff and verify `.data` exists before parsing: one page in nine failed during
the audit run, and a dropped page silently shrinks the population.

## Task 7: State file

JSON at the platform data dir. `Vec<FiredKey>` where
`FiredKey = (PrNumber, NudgeKind, HeadSha)`. Load on start, append on fire,
write atomically (temp file + rename) so a crash mid-write cannot corrupt it.

Prune entries for PRs no longer open, or the file grows forever.

## Task 8: Agent loop

`prtrack watch --repo owner/name --threshold 24h --interval 5m`

Token from `GITHUB_TOKEN`/`GH_TOKEN`, **never argv** — argv is world-readable
via `ps`. Manual `Debug` impl on the client that redacts it.

Log every evaluation decision at `debug` — while tuning the threshold you will
want to know why it stayed silent, and that is invisible otherwise.

## Task 9: `prtrack status`

Reads the state file the agent writes. Renders and exits. No raw mode, no
alternate screen, no ANSI when stdout is not a TTY.

```
prtrack — example-org/example-repo          last poll 3m ago  ·  next in 2m

  WAITING ON YOU
    #4331  22h  ▸ 2h to threshold   doc: reconcile ASC 350-60 support claims
    #4290   6h                      fix: harden aws infrastructure security

  NUDGED, STILL UNREVIEWED
    #4024  47h  nudged 23h ago      refactor: split projector persistence

  YOUR PRS, UNASSIGNED
    #4318  31h  ▸ nudged            feat: add IAS 2 period-close foundation

  4 open PRs tracked · 1 nudge fired today
```

**`last poll Nm ago` is the most important element on the screen** — it is the
liveness signal that distinguishes a quiet agent from a dead one. Render it
first, and colour it as an error past two poll intervals.

Show `▸ Nh to threshold` for PRs approaching the line. Silence should be
*explained*, not merely absent; that is what makes the threshold tunable.

Exit codes are distinct for: rendered fine, no state file (agent never ran),
and stale state (agent is dead).

## Task 10: Run it for real

Point it at `example-org/example-repo` with a deliberately low threshold (1h) for
the first hour to prove it fires at all, then set it to 24h and leave it running.

**Record, for a week:** every nudge fired, and whether the PR was reviewed
within 30 minutes after. That table is the Stage 4 kill-criteria evidence, and
it is the only reason any of this is being built.

---

## Open questions, deliberately unresolved

- Whether one nudge is enough or the tail needs escalation. Start with one;
  the data will say.
- Whether 24h is the right threshold. It fires on 15% of PRs at the measured
  baseline, which feels right, but it is a guess until observed.
- Team-wide delivery. Local notifications only reach whoever runs the agent;
  the other reviewers' p90s stay unmeasured until there is a shared channel.
