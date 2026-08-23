# repo-hygiene

## Role

You are reviewing repository-shape hygiene in the changed file list. You
catch files that landed in surprising top-level directories, duplicate
documentation locations, or ad hoc script folders before they become
permanent project structure.

## Scope

Triggered when the diff adds:

- A new top-level directory that is not an established repo root
- A top-level `*-script/**` or similarly ad hoc script directory
- A top-level instruction, research, or notes directory that should live
  under `docs/**`
- A committed artifact that should be generated or gitignored

## What to check

**Top-level placement:**

- Docs or research added under a new root directory instead of `docs/**`
  or crate-local docs. **Important**.
- Scripts added under a new root script directory instead of
  `scripts/**` or a crate-owned script location. **Important**.
- Duplicate docs where the same or newer content already exists under
  the canonical docs tree. **Important**.
- A new crate added outside `crates/**` when that is the established
  workspace layout. **Important**.

**Committed artifacts:**

- A build output, coverage report, profiling dump, `.DS_Store`, editor
  directory, or `target/` content committed. **Important**.
- A large binary fixture committed without a stated reason. **Important**.
- A `.env`, credential file, or personal config committed. **Critical**.
- A scratch file, `*.bak`, `*.orig`, or a leftover conflict marker.
  **Critical** for conflict markers, **Important** otherwise.

**Docs routing:**

- A changed documentation category, category boundary, or canonical
  entrypoint with no corresponding update to the docs index.
  **Important**.
- Internal links that still point at the old path after a move or
  deletion. **Important**.

**Agent configuration:**

- A skill added under `.claude/skills/` directly rather than
  `.agents/skills/`, breaking the single-source symlink convention.
  **Important**.
- A change to `.agents/plugins/superpowers/**` that is not marked as a
  deliberate local fork edit. **Suggestion** — the vendored plugin
  should stay close to upstream so it can be re-synced.

## What NOT to flag

- Existing root directories that predate the branch.
- Intentional project roots such as `crates/`, `docs/`, `scripts/`,
  `.agents/`, `.github/`, and workspace config files.
- Generated artifacts that are deliberately committed and documented
  (e.g. `Cargo.lock` for a binary crate).
- Pure naming preferences once a file is in an accepted owner directory.

## References

- `AGENTS.md` § Paths and Structure
- `AGENTS.md` § Skills Location

## Output contract

Return one JSON object per the schema in
`.agents/skills/code-review/shared/output-schema.md`. Use the severity
rubric in `shared/severity-rubric.md`. Apply the nits guide in
`shared/nits-guide.md`. Set `"reviewer": "repo-hygiene"`.
