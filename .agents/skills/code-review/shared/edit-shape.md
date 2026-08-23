# Edit shape — tuner output contract

The `code-review-tuner` subagent returns a single JSON object
listing proposed edits to the code-review system. Each edit is
either a minimum-diff against a playbook / orchestrator, or `null`
(diagnosis only, no edit needed).

The orchestrator and `/fix-comments` rely on this exact shape to render
edits for human triage and to apply accepted edits as a commit.

## Shape

```json
{
  "tuner": "code-review-tuner",
  "trigger_run": "<short label, e.g., 'fix-comments PR #1234' or 'retrospective on commit <sha>'>",
  "proposed_edits": [
    {
      "category": "MISSING_RULE | BAD_TRIGGER",
      "missed_finding": {
        "summary": "<one-line description of what was missed>",
        "evidence": "<the PR comment text or retrospective finding text — short, never the whole thread>",
        "file": "<repo-relative file the finding was about>",
        "line": <integer or null>
      },
      "diagnosis": "<2-3 sentences on WHY the live review missed this>",
      "edit_kind": "sharpen_existing | add_targeted_bullet",
      "target_file": "<repo-relative path to the playbook or orchestrator>",
      "diff": "<unified diff (---/+++/@@) to apply if accepted>",
      "justification": "<1 sentence linking the trigger to the proposed edit>",
      "size_delta_lines": <integer; net line delta, positive means added>,
      "dedup_check": {
        "found_similar_rule": <bool>,
        "similar_rule_excerpt": "<the existing rule that was considered, or null>"
      },
      "needs_human_review": <bool>
    }
  ],
  "no_edit_findings": [
    {
      "category": "REVIEWER_ERROR",
      "missed_finding": { /* same shape as above */ },
      "diagnosis": "<why no playbook edit is appropriate — e.g., the rule exists and fires; this was reviewer error>"
    }
  ]
}
```

## Rules

- `category` must be one of `MISSING_RULE`, `BAD_TRIGGER`, or
  (in `no_edit_findings`) `REVIEWER_ERROR`. Never any other value.
- `diff` is required for entries in `proposed_edits`. Use unified
  diff format with `---` / `+++` headers and `@@` hunks. The diff
  must apply cleanly to the current `target_file`.
- `size_delta_lines` is the net change. If positive and
  `> 0.05 × current_target_file_lines`, set `needs_human_review:
  true` and explain in `diagnosis`.
- `dedup_check.found_similar_rule` MUST be `true` when `edit_kind`
  is `sharpen_existing`; in that case `similar_rule_excerpt` is the
  rule line being sharpened.
- `evidence` cites the literal trigger text — short, never the whole
  comment thread or finding payload.
- `no_edit_findings` is for misses the tuner judged real but
  non-actionable in this loop (reviewer error, out of scope of the
  tuner, requires broader revision).
- The tuner MUST set `needs_human_review: true` if the plugin-eval
  delta gate (when available) reports a score drop after applying
  the proposed edit.

## Example

```json
{
  "tuner": "code-review-tuner",
  "trigger_run": "fix-comments PR #42",
  "proposed_edits": [
    {
      "category": "MISSING_RULE",
      "missed_finding": {
        "summary": "Poll loop spawns an unbounded task per tracked PR",
        "evidence": "PR comment: 'this spawns one task per PR every tick — 400 tracked PRs means 400 new tasks a second'",
        "file": "crates/gh-pr-tracker/src/poll.rs",
        "line": 61
      },
      "diagnosis": "The rust-core playbook covers `tokio-runtime-discipline` for blocking calls but never mentions unbounded spawning inside a repeating interval. A reviewer reading the rule would check for `block_on` and stop.",
      "edit_kind": "sharpen_existing",
      "target_file": ".agents/skills/code-review/topics/rust-core.md",
      "diff": "--- a/.agents/skills/code-review/topics/rust-core.md\n+++ b/.agents/skills/code-review/topics/rust-core.md\n@@ -30 +30 @@\n-- `block_on` called from inside an async context. **Critical**.\n+- `block_on` called from inside an async context. **Critical**. Also flag `tokio::spawn` inside a repeating interval or per-item loop with no `JoinSet`, semaphore, or bounded concurrency — task count grows with every tick. **Critical**.\n",
      "justification": "Sharpens the existing async-discipline rule to cover unbounded spawning inside the poll loop, the failure mode the PR comment surfaced.",
      "size_delta_lines": 0,
      "dedup_check": {
        "found_similar_rule": true,
        "similar_rule_excerpt": "`block_on` called from inside an async context. **Critical**."
      },
      "needs_human_review": false
    }
  ],
  "no_edit_findings": []
}
```

## What the caller does with this output

For each entry in `proposed_edits`:
1. Display the `diagnosis`, `justification`, and `diff` to the human
   via `AskUserQuestion` (or the host's prompt equivalent):
   - accept → stage the diff
   - modify → human edits the diff inline, then stage
   - reject → drop the proposal AND append to the decline log (see
     "Decline log" below). The human's freeform rejection reason is
     part of the appended entry.
2. After processing all proposals, apply staged diffs as a single
   commit with subject like:
   `chore(code-review): tune playbooks from <trigger_run> feedback`

For each entry in `no_edit_findings`:
- Log to the run report. Do NOT prompt. The human can review the log
  to decide whether reviewer-error patterns need addressing later.

## Decline log

When the human rejects a proposed edit, append one JSON line to
`.agents/skills/code-review/lib/tuner-decline-log.jsonl` with the
shape:

```json
{"timestamp": "<ISO-8601>", "trigger_run": "<as from the tuner output>", "missed_finding_summary": "<short>", "proposed_diff": "<the rejected diff>", "rejection_reason": "<the human's freeform reason; required>"}
```

The decline log is **committed to the repo** (not gitignored) so the
whole team benefits from the same accumulated learning.

The tuner reads this file at the start of every invocation and uses
it to skip semantically-similar proposals (see the tuner playbook's
"Decline-log dedup" guardrail).
