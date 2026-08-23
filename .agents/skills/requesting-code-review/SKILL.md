---
name: requesting-code-review
description: Use when completing tasks, implementing major features, or before merging to verify work meets requirements. (Project shim — delegates to code-review.)
---

# Requesting Code Review (project shim)

This is a thin project-level override of the upstream
`superpowers:requesting-code-review` skill. In this repo, all code review flows
through the centralized `code-review` skill.

## What to do

Invoke `code-review` in **inline mode** so this caller (typically a superpowers
subagent in subagent-driven-development or executing-plans) receives structured
JSON findings without any human prompt.

Map "invoke skill" to your host's mechanism:

- **Claude Code**: use the `Skill` tool with `skill: "code-review"` and
  `args: "--mode=inline"`.
- **Codex / OpenCode**: use the host's skill invocation with the same arguments.

Wait for the JSON payload. The shape is:

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

## What to do with the findings

You are a subagent finishing a task. Silently implement the fixes for all
Critical and Important findings, following the project's TDD-first rule
(failing test → implementation → passing test → commit). Suggestions may be
deferred — note them in your task summary so the orchestrator can decide whether
to triage.

Do NOT prompt the human. Do NOT print the full report. Do report a one-line
summary of what you fixed and what you deferred.

## Why this shim exists

The upstream `superpowers:requesting-code-review` dispatches a generic
code-reviewer subagent without the project-aware playbooks (Rust type design,
GitHub API rate limits and credentials, terminal-state safety, CLI contract
stability). `code-review` is the single source of truth for review in this
codebase.
