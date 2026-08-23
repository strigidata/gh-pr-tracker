# Implementer Subagent Prompt Template

Use this template when dispatching an implementer subagent.

Replace `general-purpose` with a project specialist agent when the Task Packet
matches one.

```
Task tool (general-purpose):
  description: "Implement Task N: [task name]"
  prompt: |
    You are implementing Task N: [task name]

    ## Task Packet

    - task_type: [implement | improve | review | research]
    - write_scope: [exact files/directories this subagent may edit]
    - applicable_agents_docs:
      - [AGENTS.md files the subagent must read]
    - required_skills:
      - [project-local skills the subagent must use or follow]
    - domain_boundaries:
      - [ownership rules, source-of-truth packages, and forbidden moves]
    - type_ownership:
      - [whether public types, enums, or serde contracts are in scope]
      - [where source contracts should come from]
    - verification:
      - cargo fmt --all --check
      - cargo clippy -p <crate> --all-targets --all-features
      - cargo test -p <crate>
      - [any extra focused checks expected for this task]
    - acceptance_criteria:
      - [what must be true before reporting done]

    ## Task Description

    [FULL TEXT of task from plan - paste it here, don't make subagent read file]

    ## Context

    [Scene-setting: where this fits, dependencies, architectural context]

    ## Before You Begin

    First, read every AGENTS.md file and every required project-local skill in
    the Task Packet. If any required context is unavailable, stop and
    report `NEEDS_CONTEXT`. Do not substitute global skills when a project-local
    skill exists.

    If you have questions about:
    - The requirements or acceptance criteria
    - The approach or implementation strategy
    - Dependencies or assumptions
    - Anything unclear in the task description

    **Ask them now.** Raise any concerns before starting work.

    ## Your Job

    Once you're clear on requirements:
    1. Implement exactly what the task specifies
    2. Write tests (following TDD if task says to)
    3. Verify implementation works
    4. Commit your work
    5. Self-review (see below)
    6. Report back

    Work from: [directory]

    **While you work:** If you encounter something unexpected or unclear, **ask questions**.
    It's always OK to pause and clarify. Don't guess or make assumptions.

    ## Before Reporting Back: Self-Review

    Review your work with fresh eyes. Ask yourself:

    **Completeness:**
    - Did I fully implement everything in the spec?
    - Did I miss any requirements?
    - Are there edge cases I didn't handle?

    **Quality:**
    - Is this my best work?
    - Are names clear and accurate (match what things do, not how they work)?
    - Is the code clean and maintainable?

    **Discipline:**
    - Did I avoid overbuilding (YAGNI)?
    - Did I only build what was requested?
    - Did I follow existing patterns in the codebase?
    - Did I stay inside the write scope in the Task Packet?
    - Did I follow the required AGENTS.md files and skills in the packet?
    - Did I respect crate/module ownership and type provenance?

    **Testing:**
    - Did I run `cargo test -p <crate>` and read the full output?
    - Do tests actually verify behavior (not just mock behavior)?
    - Did I follow TDD if required?
    - Are tests comprehensive, including the error and empty-result paths?

    If you find issues during self-review, fix them now before reporting.

    ## Report Format

    When done, report:
    - What you implemented
    - What you tested and test results
    - Files changed
    - How you satisfied the Task Packet
    - Self-review findings (if any)
    - Any issues or concerns
```
