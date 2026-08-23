---
name: writing-plans
description: Use when you have a spec or requirements for a multi-step task, before touching code
---

# Writing Plans

## Overview

Write comprehensive implementation plans assuming the engineer has zero context for our codebase and questionable taste. Document everything they need to know: which files to touch for each task, code, testing, docs they might need to check, how to test it. Give them the whole plan as bite-sized tasks. DRY. YAGNI. TDD. Frequent commits.

Assume they are a skilled developer, but know almost nothing about our toolset or problem domain. Assume they don't know good test design very well.

**Announce at start:** "I'm using the writing-plans skill to create the implementation plan."

**Context:** This should be run in a dedicated worktree (created by brainstorming skill).

**Save plans to:** `docs/plans/YYYY-MM-DD-<feature-name>.md`

## Investigation Gate

Do not draft implementation tasks from the spec alone. First investigate the
current code deeply enough to prove where the change belongs and what can be
reused.

1. Read the applicable `AGENTS.md` files, architecture docs, domain docs, and
   package-specific skills before choosing files or APIs.
2. Trace the current behavior end to end: entrypoint, orchestration, domain
   logic, persistence, replay/background paths, UI or API projection, and tests.
3. Search definitions and all call sites for the concepts the change needs,
   including differently named types with the same shape or responsibility.
4. Inventory existing helpers, carriers, loaders, repositories, fixtures, and
   application patterns. Classify each planned change as reuse, enrich, extract,
   replace, or genuinely new.
5. Identify the owning DDD bounded context. Domain invariants belong in that
   context; transport and persistence packages may expose scoped data but must
   not acquire consumer-specific domain behavior.
6. Map package ownership and dependency direction. Check public entrypoints,
   dependency cycles, cross-package contracts, and every affected consumer
   before proposing a new export or moving behavior.
7. Check scale-sensitive paths for query count, resident memory, cloning,
   serialization, and per-item work. Use the repository's stated production
   scale, not toy fixtures.
8. Resolve ambiguity before planning. If ownership, reuse, or the authoritative
   source of truth is unclear, stop and investigate further.

### Aggressive DRY Gate

- One domain concept gets one authoritative type and one source of truth.
- A richer carrier replaces or enriches the existing carrier. Do not keep the
  old primitive set/map beside it merely to preserve internal call signatures.
- Before adding a type, helper, query, repository, index, or fixture, cite the
  searches and existing candidates that prove none can be reused or extended.
- Prefer mechanically migrating internal callers over builders, adapters, and
  map-to-map conversions that preserve duplicate representations.
- Extract shared behavior from real callers into the owning bounded context;
  do not create a speculative shared abstraction.
- Reuse existing test fixtures and factories. Extend their domain vocabulary
  instead of creating parallel setup for the same invariant.

If a proposed item cannot pass this gate, it does not belong in the plan.

## Required Investigation Evidence In The Plan

Before the task list, record:

- **Current path:** exact symbols and files that implement the behavior today.
- **Reuse inventory:** existing types, functions, queries, fixtures, and patterns
  the implementation will reuse or enrich.
- **Duplication decision:** why every genuinely new abstraction is not a renamed
  or richer version of an existing one.
- **Bounded context:** which domain owns each invariant and why.
- **Package boundaries:** dependency direction and the public contract, if any,
  crossed by the change.
- **Scale contract:** query, memory, and per-item complexity at production size.
- **Rejected alternatives:** especially parallel carriers, duplicate queries,
  consumer-local copies, and compatibility adapters for internal-only APIs.

The evidence must cite current code, not assumptions or issue text. Re-run the
relevant searches immediately before finalizing the plan so a recent merge has
not invalidated it.

## Bite-Sized Task Granularity

**Each step is one action (2-5 minutes):**
- "Write the failing test" - step
- "Run it to make sure it fails" - step
- "Implement the minimal code to make the test pass" - step
- "Run the tests and make sure they pass" - step
- "Commit" - step

## Plan Document Header

**Every plan MUST start with this header:**

```markdown
# [Feature Name] Implementation Plan

> **For Agents:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** [One sentence describing what this builds]

**Architecture:** [2-3 sentences about approach]

**Tech Stack:** [Key technologies/libraries]

---
```

## Task Structure

````markdown
### Task N: [Component Name]

**Files:**
- Create: `exact/path/to/file.py`
- Modify: `exact/path/to/existing.py:123-145`
- Test: `tests/exact/path/to/test.py`

**Step 1: Write the failing test**

```python
def test_specific_behavior():
    result = function(input)
    assert result == expected
```

**Step 2: Run test to verify it fails**

Run: `pytest tests/path/test.py::test_name -v`
Expected: FAIL with "function not defined"

**Step 3: Write minimal implementation**

```python
def function(input):
    return expected
```

**Step 4: Run test to verify it passes**

Run: `pytest tests/path/test.py::test_name -v`
Expected: PASS

**Step 5: Commit**

```bash
git add tests/path/test.py src/path/file.py
git commit -m "feat: add specific feature"
```
````

## Remember
- Exact file paths always
- Complete code in plan (not "add validation")
- Exact commands with expected output
- Reference relevant skills with @ syntax
- Investigation evidence before task decomposition
- One authoritative carrier per domain concept
- Correct bounded context and package dependency direction
- DRY, YAGNI, TDD, frequent commits

## Execution Handoff

After saving the plan, offer execution choice:

**"Plan complete and saved to `docs/plans/<filename>.md`. Two execution options:**

**1. Subagent-Driven (this session)** - I dispatch fresh subagent per task, review between tasks, fast iteration

**2. Parallel Session (separate)** - Open new session with executing-plans, batch execution with checkpoints

**Which approach?"**

**If Subagent-Driven chosen:**
- **REQUIRED SUB-SKILL:** Use superpowers:subagent-driven-development
- Stay in this session
- Fresh subagent per task + code review

**If Parallel Session chosen:**
- Guide them to open new session in worktree
- **REQUIRED SUB-SKILL:** New session uses superpowers:executing-plans
