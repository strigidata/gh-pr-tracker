# PR nudge tool — thesis, validation plan, and product architecture

Status: **approved design, pre-implementation**
Date: 2026-08-23

## 1. Thesis

Help developers stop blocking their team: review and merge pull requests as soon
as they are ready.

The product is **the interruption**, not the dashboard. No team is unblocked
because a table was accurate. They are unblocked because someone was poked at
the right moment and acted.

This supersedes the line in `AGENTS.md` that said "the table is the product."
The table is a debugging view of the state that drives the interruption.

Two flavours of blocking, and both matter:

- **You are blocking others.** A review was requested and is untouched.
- **You are blocking yourself.** Your PR is approved and green and just sitting
  there, or it went red and you do not know yet.

Audience framing: "for lazy developers." Zero configuration, useful in thirty
seconds, and it must work when nobody is looking at it. A tool that only works
while open has close to zero value under this thesis.

## 2. What the evidence says

### The problem is real and well quantified

- Across 8.1M PRs and 4,800 teams, a third of PRs spend **78% of their
  lifecycle idle**; median cycle time 4.2 days
  ([LinearB data](https://vitalii4reva.medium.com/the-hidden-cost-of-slow-code-reviews-data-from-8-million-prs-9926849f1428)).
- Median time to first review is roughly **22 hours**
  ([Avesta](https://avestahq.com/blog/hidden-cost-of-code-reviews-22-hours-to-first-review)).
- The thesis has been stated independently, almost verbatim: *"The bottleneck is
  rarely how fast someone reviews a PR — it's how long before anyone notices the
  PR exists."*
  ([dev.to](https://dev.to/code-board/review-latency-is-a-visibility-problem-not-a-people-problem-14d3))

### The commercial picture is hostile

- **Every local desktop GitHub-PR app is free.** Neat, GitBar,
  PullRequestMenu, Trailer, PR Radar. Exactly one charges a subscription
  (MenuOps, $2.99/mo) with no visible traction. All four Mac App Store entries
  show "hasn't received enough ratings to display an overview" — the category
  is not just free, it is small.
- **GitHub bought the best product in this category and made it free.**
  [Pull Panda, June 2019](https://github.blog/2019-06-17-github-acquires-pull-panda/)
  — 1,000+ companies including Instacart and WeWork — and GitHub is
  [still investing](https://github.blog/changelog/2025-06-11-gain-more-control-and-clarity-with-scheduled-reminders-for-pull-requests/).
- **Nobody has ever publicly named a price for this.** A search across HN
  Algolia (stories and comments) plus general web found zero developers stating
  what they would pay to fix PR review latency. Not disproven — *unevidenced*,
  and it sits directly under the thesis.
- Hosted per-seat nudge tools cluster at **$2.90–$3.50/dev/mo** (PullNotifier,
  GitNotifier, PullFlow), topping out at **$8.30** for Axolo's opinionated
  workflow.
- SOC 2 Type II runs **$15–40k** in year one and is a hard gate above roughly
  50 devs. That exceeds base-case year-one ARR.

### The differentiator, stated precisely

GitHub's [personal scheduled reminders](https://docs.github.com/en/subscriptions-and-notifications/how-tos/managing-your-scheduled-reminders)
DM you in Slack about PRs awaiting your review, with optional real-time alerts
on assignment.

| | GitHub native | The gap |
|---|---|---|
| Trigger | Fixed clock times, or the event of being assigned | **Elapsed time.** "This has been waiting 6 hours" is inexpressible. |
| Scope | 5 repos per reminder, 20 oldest PRs each | Multi-repo orgs exceed this immediately |
| Direction | Review-request centric | **Author-side**: "approved and green, go merge it" |
| Channel | Slack only | OS notification, menubar, terminal |
| Suppression | None | Cooldowns, snooze, do-not-repeat |

**The first row is the product.** GitHub nudges on events and clock times. This
product nudges on **staleness thresholds**, which is the only trigger that maps
to "you are blocking someone right now."

A firehose Slack channel carrying all GitHub events is a different thing again,
and it fails for the reason the thesis predicts: it is information, not a nudge.
It is not addressed to a person, does not know whether you acted, and cannot
distinguish five minutes from three days.

## 3. What we do not know

**Whether anyone will pay.** This is the single largest risk and the reason the
plan below is validation-first rather than build-first.

Secondary unknowns:

- Does a nudge actually change behaviour, or do people mute it?
- Is the buyer the individual developer or the team lead? They have different
  price ceilings and different procurement friction.

## 4. Plan: validate before building the product

### Stage 1 — PR latency audit (first deliverable, see §5)

Measure the last ~4 months of our own repository. Produces the baseline, the
measurement instrument for every later stage, and a sales artifact.

### Stage 2 — substitution test

Every person on the team enables **personal scheduled reminder DMs plus
real-time alerts** for two weeks. Re-run the audit and compare.

Roughly 15 minutes of setup per person. Run Stage 3 interviews during the two
weeks so nothing is idle.

**Kill criterion:** if median time-to-first-review drops more than ~40%, this is
a GitHub feature and not a company. Stop, and say so out loud.

### Stage 3 — demand interviews (~10 people)

Not "would you use this." Three questions that produce revealed rather than
stated preference:

1. What have you already tried? Someone who built a bot has proven the pain.
2. Who signs off on $6/dev/month, and what would they need to see?
3. What happened the last time a PR sat for two days?

**Kill criterion:** if fewer than 3 of 10 have already built or bought
*something* for this, the pain is not sharp enough to fund.

### Stage 4 — concierge bot

A deliberately disposable Slack bot in TypeScript or Python. Nothing here
survives; do not build it in Rust.

Start with our own team for a fast iteration loop on the nudge rules. **Our own
team cannot produce a willingness-to-pay signal** — they cannot credibly refuse
and cannot be invoiced — so external teams are required before any pricing
conclusion.

Instrument the one thing that matters: per nudge type, was it acted on within 30
minutes, ignored, or muted?

**Kill criteria, fixed now so they cannot be rationalised later:**

| Signal | Threshold |
|---|---|
| Nudges acted on within 30 min | < 30% → the heuristic is noise |
| Users who mute or disable | > 20% in two weeks → notification fatigue, thesis fails |
| External teams running it 2+ weeks who will pay anything | 0 of 3 → stop or reposition |

### What carries forward from Stage 4

Three things, and only these:

1. The nudge rule set — which transitions warranted an interruption. A spec, not
   code.
2. The instrumentation schema — acted/ignored/muted per nudge type. The real
   product emits the same events.
3. Whether the buyer is the developer or the lead.

## 5. First deliverable: the PR latency audit tool

A batch CLI. No resident agent, no notifications, no TUI, no code signing, no
App Store. Fetch, compute, print.

**Written in Rust, in this repo.** It is not throwaway: it needs exactly the
layer the product needs — a GitHub provider and a normalised PR fact model — so
it is the first real brick rather than a detour. A single static binary is also
far better than a Python script for the hand-it-to-a-prospect use case.

### Metrics

Chosen so that each one maps to a nudge the product would fire. Otherwise a
before/after comparison proves nothing.

| Metric | Definition | Nudge it maps to |
|---|---|---|
| Time to first review | first review submitted − review requested (or `ready_for_review`) | "You are blocking someone" |
| **Author self-block** | merged − (approved **and** CI green) | "It is ready, go merge it" |
| Red dwell | CI failed → next push | "Your PR just went red" |
| Idle ratio | time with zero activity ÷ total lifetime | the 78% figure, for our repo |

Report **p50 and p90**, not the mean. The tail is where the pain lives.

Exclusions and edge cases: skip draft time before `ready_for_review`, exclude
bot-authored PRs, count re-review cycles separately, handle PRs closed without
merge.

**Author self-block is the metric to watch.** Nobody measures it, GitHub does
not nudge on it, and it is pure waste — the work is done and approved and just
sitting there. If that number is bad in our repo, it is the most defensible
reason for this product to exist.

### Deferred

A hosted "connect your GitHub and see how much faster you could ship" version is
attractive as a lead magnet but requires OAuth and a security review. The local
binary requires zero trust from the prospect, which directly answers the
strongest objection in the market research. Ship local first.

## 6. Product architecture (for when validation passes)

Rust owns everything that is not pixels. Each platform's resident shell embeds
the core as a library. **No daemon, no IPC, no socket.**

```
crates/
  prtrack-core/     pure logic, zero IO
                      Snapshot · Diff · Rules · Event
  prtrack-provider/ trait + normalised fact types
  prtrack-github/   GitHub provider   (prtrack-gitlab/ slots in beside it)
  prtrack-engine/   poll scheduler + credentials + cache
  prtrack-audit/    the Stage 1 batch tool
  prtrack-ffi/      UniFFI surface  ──→  macOS SwiftUI MenuBarExtra app
  prtrack-cli/      `watch` (resident) + TUI  ──→  Linux · Windows · Mac terminal
```

### Key constraints

**`prtrack-core` does no IO at all.** A pure function from (previous snapshot,
new snapshot, config, clock) to a list of events. This makes the risky part —
the nudge heuristic — testable with table-driven tests and no network, and it is
what later ports unchanged into a hosted backend.

**`Engine` is an instantiable struct with injected credentials and clock.** Never
a global, never reading env or keychain itself. The hosted tier is the revenue
path, so the core must not assume one user, one token, one machine. Cheap now,
expensive later.

**The FFI surface is tiny and synchronous:**

```
start(config, listener) -> Handle     // Rust spawns its own tokio runtime
stop(handle)
snapshot() -> Snapshot                // sync getter, returns last known state
refresh_now(handle)
```

`listener` is a Rust trait exported `with_foreign`, rendered by UniFFI as a Swift
protocol, so Rust pushes events to Swift. **No `async fn` crosses the boundary**,
which avoids the known
[UniFFI Swift 6 strict-concurrency problem](https://github.com/mozilla/uniffi-rs/issues/2274)
where async code does not conform to `Sendable`.

### Polling

The efficient loop is a cheap conditional poll plus a slow truth path:

- The REST notifications endpoint supports `If-Modified-Since`, returns
  `X-Poll-Interval`, and **304 responses do not count against the rate limit**.
  Idle developers therefore cost almost nothing.
- Notifications only cover subscribed items, so a slow full reconcile every few
  minutes is required for correctness.
- GraphQL gives PATs
  [5,000 points/hour](https://docs.github.com/en/graphql/overview/rate-limits-and-query-limits-for-the-graphql-api)
  (10,000 on Enterprise Cloud). A `gh pr status`-shaped query costs ~10–30
  points, so a 60s poll is comfortably within budget.
- `mergeStateStatus` returns `UNKNOWN` while GitHub computes mergeability. Do
  not retry-loop on it, and never render "unknown" as "conflicted."

### Platform decisions

- **macOS:** SwiftUI `MenuBarExtra` with an empty `App` body, registered via
  [`SMAppService.mainApp.register()`](https://developer.apple.com/documentation/servicemanagement/smappservice)
  for launch-at-login. The menubar app *is* the resident agent; no helper daemon.
  Read login-item status from `SMAppService` rather than storing it locally.
- **Linux / Windows:** `prtrack watch` as the resident process, plus the TUI.
- **macOS notifications require a signed app bundle** — a bare `cargo install`ed
  binary cannot reliably notify.

### Distribution

**Developer ID and direct download first; Mac App Store deferred.** MAS
discovery in this category is demonstrably weak (no listed app has enough
ratings to show a rating), and deferring it deletes the entire sandbox tax —
entitlements applied at signing, provisioning profiles, `.pkg` upload, and
[no sandboxed dev loop](https://github.com/tauri-apps/tauri/issues/15144) —
during exactly the phase where speed matters. Revisit once there is traction.

MAS also has **no free-trial mechanism for one-time purchases**, only for
subscriptions. If a one-time licence is ever sold, sell it direct.

### Pricing (provisional, revisit after Stage 4)

> Local app free forever. **$6/user/month ($60/year)** for the hosted team tier,
> free up to 3 seats and for public repos.

$6 sits above the Slack-router cluster and below Axolo's $8.30, and survives a
30% discount without falling into router pricing. Base case is roughly $32k ARR
in year one and ~$110k by year three.

Fallback if the local app must earn: $29 one-time, one year of updates, $17
renewal, sold direct — the DevUtils shape. Non-recurring, and it resets every
January.

## 7. Rejected alternatives

| Option | Why rejected |
|---|---|
| **Electron or Tauri GUI** | User decision: native Mac app, no webview. Tauri's WebKitGTK inconsistency is a real Linux tax; Electron's ~120–250MB resident footprint is poor for an always-on menubar app. Note the popular "Tauri is 10× lighter" claim is [contested](https://github.com/tauri-apps/tauri/issues/5889) — the benchmark measured RSS and ignored Chromium's shared pages. |
| **Go or TypeScript core** | Both ship faster. Rust wins only because one codebase must serve CLI, resident agent, Swift via C ABI, and a hosted backend. If the plan were "Slack in six weeks, no desktop GUI," TypeScript would be correct. |
| **Background daemon with IPC** | Unnecessary once each platform's resident shell embeds the core. No protocol to design, no lifecycle bugs. |
| **`async fn` across the FFI boundary** | Triggers the UniFFI Swift 6 `Sendable` problem. Avoided by pushing events through a foreign-implemented trait instead. |
| **swift-bridge** | Still 0.1, Swift-only, tight support window. UniFFI is production-grade for Swift with real tokio deployments. |
| **Mac App Store in v1** | Sandbox tax with no demonstrated discovery benefit in this category. |
| **Charging for the local app** | Competes with five free, maintained incumbents. Differentiation is not worth $29 against a free tool that is 90% as good. |
| **Local subscription ($2.99–3.99/mo)** | One precedent (MenuOps), no traction, and it invites the "why am I paying rent for software on my own machine using my own token" objection. |
| **Building the audit tool as a throwaway script** | It is the measurement instrument for every stage plus a sales artifact, and it needs the same provider layer the product needs. |

## 8. Known open questions

- The MAS app and a brew-installed CLI would be two distributions with separate
  credentials and separate state. Installing both means double nudges. Accepted
  for now; detect-and-warn later.
- GitLab support is intended but the provider seam will be shaped by GitHub
  alone until a second API is real. Accepted risk of a GitHub-shaped interface.
- Whether "CI green" means all checks or required checks only.
- Output format for the audit tool.
- Exact GraphQL queries and their measured point cost.

## 9. Next step

Write an implementation plan for the audit tool (§5) via the `writing-plans`
skill. Everything in §6 stays on ice until the validation stages pass.
