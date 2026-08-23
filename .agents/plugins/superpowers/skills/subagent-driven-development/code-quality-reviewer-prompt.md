# Code Quality Reviewer Prompt Template

Use this template when dispatching a code quality reviewer subagent.

**Purpose:** Verify implementation is well-built (clean, tested, maintainable)

**Only dispatch after spec compliance review passes.**

```
Task tool (superpowers:code-reviewer):
  Use template at requesting-code-review/code-reviewer.md

  WHAT_WAS_IMPLEMENTED: [from implementer's report]
  PLAN_OR_REQUIREMENTS: Task N from [plan-file]
  TASK_PACKET: [exact packet given to implementer]
  BASE_SHA: [commit before task]
  HEAD_SHA: [current commit]
  DESCRIPTION: [task summary]
```

Reviewer focus:
- Treat packet violations as code quality findings, not process trivia.
- Check crate/module ownership, type provenance, required skill guidance,
  tests, and maintainability.
- For Rust work, apply the `rust-best-practices` skill — especially the
  Illegal States and Error Handling categories.
- For terminal-rendering work, verify the render path stays allocation-light and
  degrades correctly on narrow terminals and non-TTY output.

**Code reviewer returns:** Strengths, Issues (Critical/Important/Minor), Assessment
