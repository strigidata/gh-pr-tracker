---
name: fix-comments
description: Fetch all PR review comments, fix them with a team of agents, reply on GitHub, and run full verification.
allowed-tools: Bash, Read, Write, Edit, Grep, Glob, Task, Skill, AskUserQuestion
---

# /fix-comments — Fix PR Review Comments

You are an orchestrator that fetches ALL review comments from the current GitHub
PR (including bot comments), spawns a team to fix them in parallel, replies on
each resolved comment, and runs full verification.

## Step 1: Fetch PR comments

```bash
PR_JSON=$(gh pr view --json number,headRefName)
PR_NUMBER=$(echo "$PR_JSON" | jq -r '.number')
BRANCH=$(echo "$PR_JSON" | jq -r '.headRefName')

# Code-level review comments
gh api "repos/{owner}/{repo}/pulls/$PR_NUMBER/comments" --paginate

# PR-level (issue) comments
gh api "repos/{owner}/{repo}/issues/$PR_NUMBER/comments" --paginate
```

Parse every comment and extract:

- `id` — the comment ID (needed for replying)
- `body` — the comment text
- `path` — file path (for review comments)
- `line` / `original_line` — line number
- `diff_hunk` — surrounding diff context
- `user.login` — author
- `in_reply_to_id` — thread parent (if it is a reply)
- `html_url` — link for reference

**Process every comment that still needs action**, including bot and outdated
comments — with ONE exception: **skip threads already addressed by a prior
run.** A thread is already addressed if it is resolved, or if any comment in it
carries the marker `<!-- agent-fixed -->`. Detect this with the GraphQL
`reviewThreads` API: skip any thread where `isResolved == true` OR whose joined
comment bodies contain `agent-fixed`. This is what prevents re-fixing comments
you already replied to as fixed.

**Do NOT make decisions for the user.** A technical bug or refactor is yours to
fix (following TDD). A product-level question with multiple valid options gets
analyzed and presented to the user instead of decided unilaterally.

**Do NOT leave backwards-compatibility shims** that nothing requires.

Group threaded comments together (using `in_reply_to_id`). Each thread becomes
one task.

## Step 2: Analyze and create tasks

Combine this step with the `superpowers:receiving-code-review` skill.

For each comment thread, determine whether it is **actionable** (requests a code
change, points out a bug, suggests an improvement) or **informational** (praise,
acknowledgment, question already answered).

Before deciding where to implement a requested fix, read the applicable
`AGENTS.md` files for every touched path and verify crate boundaries. If a
comment asks for normalization or validation, first determine which layer owns
that invariant. Do not implement a downstream repair when the convention says
the value must be validated at the source boundary — see `rust-best-practices`
§ `try-from-at-boundaries`.

Create a task for EVERY actionable comment thread. Include in the task:

- The full comment text (and any replies in the thread)
- The file path and line numbers
- The diff hunk for context
- The GitHub comment ID (needed for replying later)
- Whether it is a PR-level or a review comment (determines the reply API)
- How to fix it: review all tests that cover the functionality. If there are
  none, write tests that assert the desired behavior. Do not mock a type the
  project owns in order to emulate the desired behavior. If an existing test
  contradicts the requirement, use `AskUserQuestion` for clarification.

For informational comments (pure praise, acknowledgments), note them but do not
create fix tasks — you will acknowledge them on GitHub later.

**IMPORTANT**: Comments that suggest a feature improvement, propose an
alternative approach, or raise a product-level concern are NOT "informational" —
they are **decision points**. For these, use `AskUserQuestion` to present the
options and let the user decide before replying on GitHub. Never unilaterally
dismiss a comment as "out of scope" or "for a follow-up" without asking first.

## Step 3: Spawn a team and fix comments

Spawn worker agents in parallel.

**Worker assignment strategy:**

- 1–3 tasks → 1–2 workers
- 4–8 tasks → 2–3 workers
- 9+ tasks → 3–4 workers (max)

Give each worker the **investigate-then-fix** iron law:

1. Read the task description to understand the requested change.
2. Read the relevant file(s) for full context.
3. **Investigate before fixing**: write a 2-sentence root-cause analysis
   identifying WHY the issue exists (what assumption was wrong, what edge case
   was missed, what contract was broken). This becomes part of the commit body.
4. **TDD**: write a failing test capturing the requested behavior, then
   implement the minimum fix to make it pass.
5. Make the fix, following `AGENTS.md` and `rust-best-practices`.
6. Mark the task completed.

The root-cause step is non-negotiable. A worker that proceeds to fix without
writing the root cause must stop and report `NEEDS_CONTEXT` so the orchestrator
can re-dispatch with more guidance. This prevents cargo-cult fixes that paper
over symptoms.

Assign workers so that no two touch the same file concurrently. Two agents
editing one `.rs` file will conflict.

Wait for all workers to finish.

## Step 4: Reply on GitHub

For **review comments** (code-level), reply in the same thread:

```bash
gh api "repos/{owner}/{repo}/pulls/$PR_NUMBER/comments" \
  --method POST \
  -f body="Fixed — <brief description of the change> <!-- agent-fixed -->" \
  -F in_reply_to=<ORIGINAL_COMMENT_ID>
```

For **PR-level (issue) comments**:

```bash
gh api "repos/{owner}/{repo}/issues/$PR_NUMBER/comments" \
  --method POST \
  -f body="Fixed — <brief description of the change> <!-- agent-fixed -->"
```

For non-actionable comments, reply with a brief acknowledgment.

Keep replies to 1–2 sentences describing what was fixed and where. Every "Fixed"
reply MUST include the `<!-- agent-fixed -->` marker so subsequent runs
recognize the thread as addressed and skip it (see Step 1). Do not resolve the
thread — leave that to the reviewer.

## Step 5: Commit changes

```bash
git add -A
git commit -m "fix: address PR review comments

<bullet list of changes made>
"
```

## Step 5.5: Tuner feedback loop (improve the review system)

Before verification, feed the missed-comment signals back into the review system
so future PRs catch them automatically.

1. **Collect missed findings** from the actionable threads processed in Step 3.
   For each: `summary` (one line), `evidence` (the literal comment text, short),
   `file`, `line`. Skip threads classified as nits, pure praise, or decision
   points the human already resolved interactively.
2. **If the list is empty**, skip the rest of this step.
3. **Dispatch the tuner subagent**:
   - **Brief**: the verbatim contents of
     `.agents/skills/code-review/topics/tuner.md`, plus
     `shared/severity-rubric.md`, `shared/nits-guide.md`, and
     `shared/edit-shape.md`.
   - **Input**: the missed-findings list, plus the current contents of every
     playbook in `.agents/skills/code-review/topics/` and the orchestrator
     `SKILL.md`.
   - **Instruction**: classify each missed finding into `MISSING_RULE` /
     `BAD_TRIGGER` / `REVIEWER_ERROR`. Return a single JSON object matching
     `shared/edit-shape.md`.
4. **Human triage of proposed edits**: for each entry in `proposed_edits`,
   render the `diagnosis`, `justification`, and `diff` and ask via
   `AskUserQuestion`:
   - `accept` → stage the diff
   - `modify` → human edits the diff inline, then stage
   - `reject` → ask for a one-line reason and append a JSON line to
     `.agents/skills/code-review/lib/tuner-decline-log.jsonl` per
     `shared/edit-shape.md`

   For `no_edit_findings`, log to the run report but do NOT prompt.
5. **Apply accepted edits AND decline-log appends** as a single commit BEFORE
   verification:
   ```bash
   git add .agents/skills/code-review/
   git commit -m "chore(code-review): tune playbooks from PR #$PR_NUMBER feedback"
   ```

This keeps the tuner's improvements separate from the human-comment fixes and
lets reviewers of THIS PR see exactly which playbook changes the feedback loop
produced.

## Step 6: Run verification

Invoke the `verify` skill to run the full suite.

If verification fails:

1. Fix the failing step.
2. Amend the commit or create a new fix commit.
3. Re-run `verify` until everything passes.

## Step 7: Push and report

```bash
git push
```

Report to the user:

- Number of comments addressed
- What was changed
- Verification results
- Link to the PR

## Important Notes

- **Do NOT skip bot comments** — they may contain valid review feedback.
- **Do NOT resolve or dismiss** review threads — only reply to them.
- Always read the full file before making changes.
- Follow existing patterns and `AGENTS.md` conventions.
- If a comment is ambiguous, make your best judgment and note it in the reply.
- If a comment requires a design decision, presents multiple valid approaches,
  or suggests a feature improvement — **ask the user via `AskUserQuestion`**
  before deciding. Never unilaterally dismiss as "out of scope".
