# Output schema — what every reviewer returns

Every reviewer subagent **must** return a single JSON object matching
the shape below. The aggregator parses these payloads, merges them by
`(file, line, title)`, sorts by severity, and emits the final report.

## Shape

```json
{
  "reviewer": "<reviewer-name>",
  "model": "<claude-sonnet-5 | gpt-6.1-sol | other model id>",
  "note": "<optional diagnostic string from the dispatch helper or reviewer; empty/omitted by default>",
  "findings": [
    {
      "severity": "Critical | Important | Suggestion",
      "file": "<repo-relative path>",
      "line": <integer line number>,
      "title": "<short imperative phrase, ≤ 90 chars>",
      "evidence": "<2-6 lines of code/diff context>",
      "why_it_matters": "<1-2 sentences linking to a tenet, a rust-best-practices rule, or an AGENTS.md rule>",
      "what_breaks": "<REQUIRED for Critical: one concrete sentence describing what fails in production if this ships. Optional for Important. Omit for Suggestion.>",
      "proposed_change": "<concrete edit; required for Suggestion, optional for Important/Critical>",
      "references": ["<docs path, AGENTS.md §, or rust-best-practices rule id>", "..."]
    }
  ]
}
```

The optional top-level `note` field is used by the cross-model dispatch
helpers (`lib/dispatch-{codex,claude}-reviewer.sh`) to convey graceful-
degradation messages when the target CLI is missing or fails (e.g.,
`"note": "Codex CLI not installed; cross-model second opinion
skipped."`). Reviewers may also use it for non-finding diagnostics
(timeouts, partial coverage). The aggregator surfaces non-empty `note`
fields in the run report so the human knows when a second opinion
wasn't actually delivered.

## Rules

- `reviewer` must match the playbook filename (without `.md`).
- `model` identifies which model produced these findings (e.g.,
  `claude-sonnet-5` or `gpt-6.1-sol`). The aggregator uses this to compute
  unanimous-vs-single-model findings when cross-model review is on.
- `findings` may be empty: `{ "reviewer": "...", "model": "...", "findings": [] }`.
- `severity` must be one of `Critical`, `Important`, `Suggestion`.
  **Never** emit `Nit`.
- `title` is an imperative phrase ("Unwrap on GitHub API response", not
  "There's an unwrap that ...").
- `evidence` is short — paste the offending lines, not the whole file.
- **`what_breaks` is REQUIRED for every `Critical` finding.** It must
  be ONE concrete causal sentence describing what fails in production
  if the change ships unfixed. Not "this is bad" or "could cause
  problems" — name the failure (e.g., "A 502 from the GitHub API
  panics the poll task, the live table freezes on stale data, and the
  user sees no error because the panic is swallowed by the join
  handle"). The aggregator downgrades any `Critical` lacking
  `what_breaks` to `Important` and flags the reviewer for retraining
  via the tuner.
- `references` cite specific tenets, files, or rules. Empty array is OK.

## Example

```json
{
  "reviewer": "rust-core",
  "model": "claude-sonnet-5",
  "findings": [
    {
      "severity": "Critical",
      "file": "crates/gh-pr-tracker/src/github/client.rs",
      "line": 87,
      "title": "Unwrap on GitHub API response body",
      "evidence": "let prs: Vec<PullRequest> = resp.json().await.unwrap();",
      "why_it_matters": "`no-unwrap-in-prod`: a deserialization failure or a rate-limit HTML error page panics the polling task instead of surfacing a recoverable error.",
      "what_breaks": "When GitHub returns a rate-limit or maintenance page instead of JSON, the poll task panics; the live table silently stops updating and keeps rendering stale PR state indefinitely.",
      "proposed_change": "Return `Result<Vec<PullRequest>, GithubError>` and propagate with `?`, mapping the serde error into a `GithubError::Decode { source }` variant so the TUI can render a retry banner.",
      "references": ["rust-best-practices § no-unwrap-in-prod", "AGENTS.md § Working Style"]
    }
  ]
}
```

## Aggregator behavior (informational; reviewers don't implement it)

- Merge across reviewers by exact `(file, line, title)` match.
- Sort: Critical → Important → Suggestion.
- **Downgrade Criticals that lack `what_breaks`** to Important; record
  the downgrade in the run report so the tuner can propose a reviewer-
  retraining edit on the next loop iteration.
- Apply [`./nits-guide.md`](./nits-guide.md) — anything that should be a
  Nit is dropped. Reviewers SHOULD have already suppressed; aggregator
  is the last line of defense.
- **Cross-model merging** (when the high-risk auto-trigger fires): for
  each finding key `(file, line, title)`, set `consensus: "unanimous"`
  if both models flagged it, else `consensus: "single-model"` and
  record which model. Surface this in the report so humans see which
  Criticals both models agreed on.
- Emit one markdown table per severity group in interactive mode, or a
  flat JSON array in inline mode.
