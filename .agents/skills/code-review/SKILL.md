---
name: code-review
description: Run the centralized code-review pipeline on the current branch diff. Dispatches specialized reviewer subagents in parallel (domain reviewers plus cross-cutting concerns), aggregates findings by severity, and reports or acts based on mode. Use as the soft gate before /verify in the Definition of Done, or as the post-task review step in superpowers subagent-driven-development.
allowed-tools: Bash, Read, Grep, Glob, Task, Skill, AskUserQuestion
---

# /code-review — Centralized Code Review

Run the canonical code review on the current branch.

This skill is a mandatory multi-agent review gate. Invoking `/code-review`
or reaching the DoD step that requires `/code-review` is explicit
authorization to dispatch the reviewer subagents described below. Do not fall
back to a local-only review because generic agent-tool guidance says not to
spawn agents unless the user asks; this skill is the project-level request.

This skill is invoked from two places:

- **Interactive mode** (`--mode=interactive`, the default): a human typed
  `/code-review`, or this is running as the Definition of Done gate before
  `/verify`. Report findings to the human; offer to dispatch a fix team.
- **Inline mode** (`--mode=inline`): another agent (typically a superpowers
  subagent in subagent-driven-development or executing-plans) called this
  skill to get a structured review of recent changes. Return findings as
  JSON; do not prompt the human.

## Step 1 — Pick the base ref

Source the shared helper, then compute the review-only diff window.

The `github-stacked-pr` skill sets `CR_BASE_REF_OVERRIDE` to the immediate
parent branch for a layer review. Do not set or guess this override outside a
workflow that has already validated native stack topology.

```bash
source .agents/skills/code-review/lib/base-ref.sh
CODE_REVIEW_BASE_REF="$BASE_REF"
CODE_REVIEW_HISTORY_BASE_REF=$(git merge-base origin/main HEAD)

export CODE_REVIEW_BASE_REF CODE_REVIEW_HISTORY_BASE_REF
echo "code-review base ref: $CODE_REVIEW_BASE_REF"
echo "code-review history base ref: $CODE_REVIEW_HISTORY_BASE_REF"
```

When `CR_BASE_REF_OVERRIDE` is set, the helper fails unless that ref exists and
is an ancestor of `HEAD`. Otherwise, it uses `origin/<current-branch>` only for
a clean pushed checkpoint. If that range contains files inherited unchanged
from an unpushed `origin/main` merge, it uses `origin/main` so main-only code is
excluded from review.

Before routing reviewers, write a concise **branch objective** from the current
user request, linked issue, accepted plan, and PR description. State only
requirements supported by those sources. If none is available, write
`objective unavailable`; do not infer one from the implementation.

## Step 2 — List and bucket changed files

```bash
CHANGED_FILES=$(mktemp -t code-review-changed-XXXXXX)
trap 'rm -f "$CHANGED_FILES"' EXIT
git diff --name-only --diff-filter=ACDMRTUXB "$CODE_REVIEW_BASE_REF...HEAD" > "$CHANGED_FILES"
wc -l "$CHANGED_FILES"
```

`--diff-filter=ACDMRTUXB` keeps deleted paths for reviewer routing so removal
diffs still get reviewed. When building reviewer context, use `git diff` for
deleted files and only `Read` paths that still exist in the worktree. `mktemp`
prevents parallel worktrees from clobbering each other's lists; the EXIT trap
removes the temp file when the orchestrator exits.

Bucket each path:

| Bucket            | Matches |
| ----------------- | ------- |
| **docs-only**     | `*.md`, `docs/**`, `AGENTS.md`, `CLAUDE.md`, `.agents/**`, `.claude/**`, `.codex/**` |
| **workflow-only** | `.github/workflows/**`, `.github/actions/**`, issue/PR templates, label configs |
| **code-mixed**    | `crates/**`, `src/**`, `tests/**`, `benches/**`, `examples/**`, `Cargo.toml`, `Cargo.lock`, `build.rs`, `rust-toolchain.toml`, `.cargo/**`, release config, `Dockerfile*`, OR a mix that includes any of these |

Routing:

- **docs-only** → if the changed files include suspicious top-level additions,
  fire only the `repo-hygiene` reviewer. If they change documented behavior
  (README usage, flag lists, output samples), also fire `comments-docs`.
  Otherwise exit clean and report "no code review needed for docs-only change."
- **workflow-only** → fire only the `infra` reviewer.
- **code-mixed** → proceed to Step 3.

## Step 3 — Compute the reviewer matrix

For each **domain reviewer**, fire it if any changed file matches its globs.
Every `*.rs` path in the repo must match at least one reviewer; if a new crate
or module layout is added, extend this table.

| Reviewer      | Globs |
| ------------- | ----- |
| `rust-core`   | All `**/*.rs`, `Cargo.toml`, `Cargo.lock`, `build.rs` — MINUS paths owned by a more specific reviewer below (those still get `rust-core` too when the change is not purely domain-specific; when in doubt, fire both) |
| `github-api`  | `**/github/**`, `**/api/**`, `**/client/**`, `**/gh/**`, any `.rs` file whose diff imports an HTTP/GraphQL client or shells out to `gh` |
| `tui-render`  | `**/render/**`, `**/ui/**`, `**/tui/**`, `**/table/**`, `**/view/**`, any `.rs` file whose diff imports a terminal/TUI crate |
| `cli-ux`      | `**/main.rs`, `**/cli.rs`, `**/args.rs`, `**/config.rs`, `**/commands/**`, shell-completion generation |
| `public-api`  | `**/lib.rs`, any diff hunk adding or changing a `pub` item reachable from a library crate root, `[features]` in `Cargo.toml` |
| `infra`       | `.github/workflows/**`, `.github/actions/**`, `Dockerfile*`, `docker-compose*.{yml,yaml}`, `rust-toolchain.toml`, `.cargo/config.toml`, release config |

For each **cross-cutting reviewer**, do a **two-stage** check:

1. **Cheap stage**: glob match against changed files.
2. **Signal stage**: if no glob match, grep the diff content for trigger patterns.

| Reviewer           | Globs | Diff-content triggers |
| ------------------ | ----- | --------------------- |
| `branch-artifacts` | **Always fire for code-mixed reviews.** Review the changed files and use their earlier revisions as context. | — |
| `tests`            | `**/tests/**`, `**/benches/**`, fixture/cassette/snapshot changes, `[dev-dependencies]` | added `#[test]`, `#[tokio::test]`, `#[cfg(test)]`, `mod tests` |
| `observability`    | — | new `tracing::`/`log::` call, new `eprintln!`/`println!`, new `.ok()`/`unwrap_or_default()`/`let _ =` on a fallible call, new error type or variant, new retry/timeout/fallback |
| `comments-docs`    | `README.md`, `docs/**` | added or removed `///` or `//!` blocks, OR a diff touching an exported declaration line — match `^[+-]\s*pub\b` to cover `pub fn`, `pub struct`, `pub enum`, `pub trait`, `pub use`, etc. The doc comment above may now be stale |
| `repo-hygiene`     | added paths in a new top-level directory, top-level `*-script/**`, committed artifacts | added root-level docs or scripts that should live under `docs/**` or `scripts/**` |

Compute the diff content once with `git diff "$CODE_REVIEW_BASE_REF...HEAD"` and
reuse it. Never route or review files that disappear from this review-only diff
solely because they were merged from `origin/main`.

For `branch-artifacts`, also collect the non-merge history only for paths in
`CHANGED_FILES`:

```bash
git log --no-merges --reverse --name-status \
  --format='commit %H%nsubject %s' \
  "$CODE_REVIEW_HISTORY_BASE_REF..HEAD" \
  -- <changed-files...>
```

**High-risk detection.** If the fired reviewer set includes ANY of
`github-api`, `public-api`, or `infra`, set `HIGH_RISK=true`. These carry the
highest cost of a missed finding: credential handling and rate-limit
correctness, an external contract that cannot be refactored alongside you, and
supply-chain/release exposure. For each high-risk reviewer, Step 4 dispatches
BOTH a primary and a cross-model second-opinion reviewer in parallel.
Cross-model review catches model-specific blind spots where the cost of being
wrong is highest.

## Step 4 — Dispatch reviewers in one parallel wave

For each fired reviewer, dispatch a subagent with:

- **Model**: `claude-sonnet-5` on Claude Code; `gpt-5.5` (fallback `gpt-5.4`) on Codex.
- **Slice**: the changed files matching the reviewer's globs (or all changed
  files for trigger-only reviewers like `observability`).
- **Brief**: the verbatim contents of:
  - `.agents/skills/code-review/topics/<reviewer>.md`
  - `.agents/skills/code-review/shared/severity-rubric.md`
  - `.agents/skills/code-review/shared/nits-guide.md`
  - `.agents/skills/code-review/shared/output-schema.md`
- **Diff context**: `git diff "$CODE_REVIEW_BASE_REF...HEAD" -- <files...>`. For
  deleted files this diff is the source of truth; do not try to read them as
  live worktree files.
- **Implementation-lineage context for `branch-artifacts` only**: the branch
  objective, the diff, and the file-scoped non-merge history. It may use
  `git show` and repository search to understand earlier versions.
- **Delegated skills**: if the playbook's YAML frontmatter has a `delegates:`
  list, the reviewer subagent MUST load each named skill via the Skill tool
  before flagging language-level concerns. The delegated skill is the canonical
  guidance for generic patterns; the playbook covers only project-specific
  rules. If there is no `delegates:` frontmatter, this step is a no-op.
- **Instruction**: read the listed files as needed for full context, then
  return findings as a single JSON object matching `output-schema.md`. Set
  `"model"` to whatever model the host actually used. Do not perform fixes;
  report only. Reviewers judge technical risk in the supplied diff. Except for
  `branch-artifacts`, they do not know the user's original task, issue scope, or
  current authorization. The `branch-artifacts` reviewer receives the objective
  only to identify remnants of superseded approaches; it still must not
  authorize fixes or make scope decisions.

**Cross-model second opinion (auto-fires when `HIGH_RISK=true`).** The second
opinion is the OTHER model from your current host — never the same model twice.

- Running under **Claude Code** (primary = Sonnet 5) → dispatch the second
  opinion to GPT-5.5 via `lib/dispatch-codex-reviewer.sh`.
- Running under **Codex** (primary = GPT-5.5) → dispatch the second opinion to
  Sonnet 5 via `lib/dispatch-claude-reviewer.sh`.
- Under another host, pick whichever helper points at a model different from
  your own.

```bash
BRIEF=$(mktemp -t cross-model-brief-XXXXXX)
cat .agents/skills/code-review/topics/<reviewer>.md \
    .agents/skills/code-review/shared/severity-rubric.md \
    .agents/skills/code-review/shared/nits-guide.md \
    .agents/skills/code-review/shared/output-schema.md \
    > "$BRIEF"

SLICE_DIFF=$(mktemp -t cross-model-diff-XXXXXX)
git diff "$CODE_REVIEW_BASE_REF...HEAD" -- <files-in-this-slice> > "$SLICE_DIFF"

SECOND_OPINION_OUT=$(mktemp -t cross-model-out-XXXXXX)

bash .agents/skills/code-review/lib/dispatch-<other-host>-reviewer.sh \
  <reviewer> "$BRIEF" "$SLICE_DIFF" "$SECOND_OPINION_OUT"
```

The helper returns a JSON payload conforming to `output-schema.md` with
`"model"` set to the second-opinion model name (or `<model>-unavailable` /
`<model>-error` for graceful degradation when the CLI is missing or fails).
Include the helper output in the aggregator's input alongside the primary
outputs.

Dispatch all reviewer subagents **in one message**, in parallel. Wait for all to
return.

## Step 5 — Aggregate

Collect every reviewer's JSON (including both models' outputs for high-risk
reviewers). Merge across reviewers by exact `(file, line, title)`. Sort by
severity: Critical → Important → Suggestion. Apply the nits guide one more time.

**Cross-model consensus.** For each finding key `(file, line, title)` that
appears in BOTH the primary and the second-opinion output for the same
reviewer, set `consensus: "unanimous"`. For single-model findings, set
`consensus: "single-model"` with the model name. Surface this in the
interactive report so humans can see which Criticals both models agreed on
(high confidence) versus which warrant closer inspection.

**Critical without `what_breaks` downgrade.** Any `Critical` finding lacking the
required `what_breaks` field is downgraded to `Important`, and the reviewer name
is recorded in a sidecar `aggregator_notes.downgraded_reviewers` field for the
tuner to act on next loop iteration.

Emit a single report.

## Step 5.5 — Main-thread scope triage

Reviewer findings are not authorization to edit. Reviewers only know the
technical diff. The orchestrating agent in the main thread must compare each
Critical and Important finding against the user's stated task, linked issue,
branch objective, and any scope clarifications in the conversation before
deciding what can be fixed in this PR.

What is "in scope":

- The original GitHub issue or user prompt
- Design docs and implementation-plan requirements and tasks
- Anything else the user explicitly asked for

Classify every Critical and Important finding into one of:

| Classification | Meaning | Allowed action |
| -------------- | ------- | -------------- |
| `authorized_fix` | Directly required by the user's task, the linked issue, or an explicit clarification. | May fix in this PR. |
| `needs_user_confirmation` | Branch-owned or adjacent to touched code, but not clearly authorized. | Ask the user before fixing. |
| `follow_up_only` | Valid technical concern outside the current task or PR objective. | Report as a proposed follow-up. Do not fix. |
| `reject` | Not applicable after main-thread context, stale, or contradicted by surrounding code. | Drop and explain briefly if needed. |

Suggestions are never automatic edit authorization. Include them in the report
only when useful, or convert them to proposed follow-ups.

If a finding is `needs_user_confirmation` or `follow_up_only`, do not route it
to a fix team and do not implement it locally unless the user explicitly
approves. Critical severity changes urgency; it does not expand the authorized
scope by itself.

## Step 6 — Mode-dependent finish

### Interactive mode (default)

Render the report as markdown grouped by severity:

```markdown
# code-review report — base `$CODE_REVIEW_BASE_REF`...HEAD

Fired: <N> reviewers, <M> returned findings.

## Critical (<count>)

| File:Line | Title | Reviewer | Why it matters |
| --------- | ----- | -------- | -------------- |
| ...       | ...   | ...      | ...            |

## Important (<count>)

| ... |

## Suggestion (<count>)

| ... |
```

After scope triage, render the Critical and Important findings in separate
groups:

- **Authorized fixes** — `authorized_fix`
- **Needs confirmation** — `needs_user_confirmation`
- **Proposed follow-ups** — `follow_up_only`
- **Rejected by context** — omit by default unless the rejection is likely to
  be surprising

If there are authorized Critical or Important findings, ask:

> Dispatch fix team for N authorized Critical + M authorized Important findings? (yes / no / triage)

- **yes** → dispatch parallel fix subagents only for `authorized_fix` findings
  (one per file with grouped findings). Each subagent follows
  **investigate-then-TDD**: (1) write a 2-sentence root-cause analysis in the
  commit body explaining WHY the finding exists and what assumption was wrong,
  (2) write the failing test, (3) implement the fix, (4) run the test,
  (5) commit. The root-cause requirement is the "no fix without investigation"
  iron law — it prevents cargo-cult fixes that paper over the symptom. Then run
  the **fix-team retrospective** (Step 6.5) before suggesting a re-run.
- **no** → return; the human will fix manually.
- **triage** → ask the human, finding by finding, accept/defer/decline.
  Human-accepted findings become `authorized_fix`. Deferred findings become
  proposed follow-ups; do not create GitHub issues without explicit
  confirmation.

If there are no authorized Critical or Important findings but there are
`needs_user_confirmation` findings, ask whether any should become authorized
fixes for this PR. Otherwise, report them as proposed follow-ups and proceed to
`/verify` for the authorized scope only.

## Step 6.5 — Fix-team retrospective (interactive mode only, after `yes`)

The retrospective is the feedback loop that improves playbooks over time.

1. **Re-source `$BASE_REF`** and recompute `$CODE_REVIEW_BASE_REF` so it picks
   up the fix team's new commits.
2. **Re-run Steps 2–5** on the new diff.
3. **Identify retrospective signals**: any finding whose `file` matches a file
   the fix team touched AND whose `line` falls in a hunk the fix team added or
   modified. These are issues the original wave should have flagged.
4. **If retrospective signals exist**, dispatch the **tuner subagent**:
   - **Brief**: the verbatim contents of
     `.agents/skills/code-review/topics/tuner.md`, plus
     `shared/severity-rubric.md`, `shared/nits-guide.md`, and
     `shared/edit-shape.md`.
   - **Input**: the retrospective signals (file/line/title/evidence), plus the
     current contents of every playbook in `topics/` and this `SKILL.md`.
   - **Instruction**: classify each missed finding into `MISSING_RULE` /
     `BAD_TRIGGER` / `REVIEWER_ERROR`. Return a single JSON object matching
     `shared/edit-shape.md`.
5. **Human triage of proposed edits**: for each entry in `proposed_edits`,
   render the `diagnosis`, `justification`, and `diff` and ask via
   `AskUserQuestion` (accept / modify / reject). On **reject**, ALSO ask for a
   one-line reason and append a JSON line to
   `.agents/skills/code-review/lib/tuner-decline-log.jsonl` per the shape in
   `shared/edit-shape.md`. For `no_edit_findings`, log to the run report but do
   not prompt.
6. **Apply accepted edits**: stage the accepted diffs PLUS any new decline-log
   lines, then commit them together:
   ```
   chore(code-review): tune playbooks from retrospective on <fix-team-commit-sha>
   ```
7. Suggest re-running `/code-review` on the final state.

If there are no retrospective signals, skip Step 6.5 and proceed to `/verify`.

If there are zero authorized Critical/Important findings and at most a few
Suggestions or proposed follow-ups, print:

> Review clean for the authorized scope. Proceed to `/verify`.

### Inline mode

Skip the markdown render. Return a single JSON payload to the calling agent:

```json
{
  "base_ref": "<value of $CODE_REVIEW_BASE_REF>",
  "summary": {
    "critical": 0,
    "important": 0,
    "suggestion": 0,
    "reviewers_fired": 0,
    "reviewers_with_findings": 0
  },
  "findings": [],
  "partial": false
}
```

`partial` is `true` if any reviewer subagent failed and the run continued
without it.

The calling agent decides what to fix, but it must run Step 5.5 in the main
thread before editing. Inline reviewer output is technical input only and is
not edit authorization. No human prompt happens inside inline mode.

## Adding or changing a playbook

Every playbook in `topics/` must have the sections `## Role`, `## Scope`,
`## What to check`, `## What NOT to flag`, and `## Output contract`. Validate
before committing:

```bash
bash .agents/skills/code-review/lib/validate-playbook.sh .agents/skills/code-review/topics/<name>.md
```

When you add a reviewer, add its row to the Step 3 matrix in the same commit.
A playbook that no glob or trigger routes to never fires.

## Cross-tool notes

- This SKILL.md uses Claude Code's `Task`, `Skill`, and `AskUserQuestion` tools.
  On Codex/OpenCode/Cursor, map "dispatch a subagent" to the host's native
  subagent mechanism, and map "ask the user" to the host's native prompt.
- Reviewer playbooks under `topics/**` and shared assets under `shared/**` are
  tool-neutral markdown.
- `lib/base-ref.sh` is plain bash, sourced identically by every host.
