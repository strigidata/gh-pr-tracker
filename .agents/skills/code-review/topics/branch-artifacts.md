# branch-artifacts

## Role

You review implementation lineage for remnants of abandoned intermediate
solutions that remain technically valid but no longer serve the final branch
objective.

## Scope

Always run for a code-mixed review. Use the branch objective, diff, and
non-merge history supplied by the orchestrator.

Review branch-introduced or branch-moved structures and branch-touched
declarations. Inspect earlier versions when needed to determine why a structure
was introduced and whether that reason still exists.

## What to check

**Abandoned implementation structure:**

- File relocation, public-symbol rename, provider, adapter, or abstraction
  introduced for an intermediate approach whose consumer path was later
  removed or replaced. **Important**.
- Branch-added alias, shim, fallback, or compatibility path with no live caller,
  persisted contract, rollout requirement, or documented support boundary.
  **Important**.
- Branch-added helper or fixture kept only because another branch-added file
  still imports it, when both preserve structure from the abandoned approach.
  **Important**.

**Final semantic consistency:**

- Identifier, test name, fixture name, or comment that describes an obsolete
  subset of the final behavior. Examples include `single_repo` after multi-repo
  support was added and `poll_once` after continuous refresh was added.
  **Important**. Do not suppress this as a naming nit.
- Test or generated artifact that verifies or represents an intermediate design
  rather than the final supported behavior. **Important**.
- Generated output left by a superseded input, generator, or deleted scenario.
  **Important**.

**Retention evidence:**

For anything that looks like compatibility or intentional indirection, search
for the concrete retention reason. Accept only a current consumer, external or
persisted contract, rollout boundary, ownership boundary, or canonical
generator requirement. A consumer counts only when it requires the retained
relocation, abstraction, or indirection. A rewritten import, a branch-added
consumer that merely mirrors the abandoned structure, or successful compilation
is not retention evidence. If concrete evidence exists, do not flag it.

## What NOT to flag

- A structure merely because a different design would be cleaner.
- Cosmetic renames that do not contradict final behavior.
- Pre-existing legacy code not introduced or moved by the branch.
- Compatibility code with a verified live contract or rollout reason.
- Required generated outputs, including committed lockfiles and generated
  API bindings.
- Unrelated main-branch changes inherited through a merge.

## Output contract

Return one JSON object per the schema in
`.agents/skills/code-review/shared/output-schema.md`. Use the severity rubric
in `shared/severity-rubric.md`. Apply the nits guide in
`shared/nits-guide.md`. Set `"reviewer": "branch-artifacts"`.
