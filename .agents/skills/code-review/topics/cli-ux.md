---
delegates:
  - rust-best-practices
---

# cli-ux

## Role

You are reviewing the command-line surface: argument and subcommand
definitions, configuration loading and precedence, exit codes, the
stdout/stderr split, and machine-readable output. This surface is a
contract with the user's shell scripts, so changing it breaks things
that never get tested.

You do NOT review terminal rendering (`tui-render`) or GitHub requests
(`github-api`).

## Scope

- `main.rs`, `cli.rs`, `args.rs`, `config.rs`, `commands/**`
- Clap/argh derives, config file structs, env-var reads
- Shell-completion and man-page generation

## What to check

**Contract stability:**

- A flag, subcommand, or positional argument removed or renamed without
  a deprecation alias. **Critical** — it silently breaks user scripts.
- A flag's default value changed. **Critical** unless the change is the
  stated purpose of the branch.
- A flag's type widened or narrowed (single value → multiple, string →
  enum) in a way that rejects previously valid input. **Critical**.
- A new required argument added to an existing command. **Critical**.

**Config precedence:**

- Precedence that is not `defaults < config file < environment <
  flags`, or that is implemented inconsistently across settings.
  **Important** — state the observed order.
- A setting reachable by flag but not by config file (or vice versa)
  with no stated reason. **Suggestion**.
- Config parsed with unknown fields silently ignored. **Important** — a
  typo'd key should be an error, not a silent no-op.
- Config or cache written to a hardcoded `~/.gh-pr-tracker` instead of
  the platform config/cache directories. **Important**.

**Exit codes and streams:**

- Every failure path collapsed to `exit(1)`, or a failure that exits
  `0`. **Critical** — distinguish at minimum: success, no results,
  usage error, auth failure, network/API failure.
- A `main` that returns `Result` and lets the default `Debug`-formatted
  error reach the user. **Important** — print a human-readable error
  chain to stderr.
- Diagnostics, progress, or the interactive table written to stdout
  when stdout is the data channel. **Critical** — it corrupts pipes.
  Human-facing chatter belongs on stderr.
- A panic used as the exit path for an expected user error (bad repo
  name, missing token). **Critical**.

**Machine-readable output:**

- A `--json` (or equivalent) mode whose output includes ANSI codes, a
  spinner, or a human header. **Critical**.
- JSON output whose shape changes between the empty case and the
  populated case (`{}` vs `[]` vs `null`). **Important**.
- No `--json`/`--format` escape hatch on a tool whose whole purpose is
  aggregating state other tools will want. **Suggestion**.

**Help and discoverability:**

- A new flag with no `about`/help text, or help text that restates the
  flag name. **Important**.
- Help text that documents behavior the code does not implement.
  **Critical** — this is comment rot in the user-facing surface.
- An interactive prompt or a TUI launched when stdin/stdout is not a
  TTY, with no non-interactive fallback. **Critical** — it hangs in CI.

## What NOT to flag

- Anything `cargo clippy` catches — CI handles it.
- Flag-naming preferences that are consistent with existing flags.
- Choice of argument-parsing crate.
- Pre-existing commands the branch did not touch.

## External references (optional)

- `AGENTS.md` § CLI contract
- `rust-best-practices` § Error Handling

## Output contract

Return one JSON object per the schema in
`.agents/skills/code-review/shared/output-schema.md`. Use the severity
rubric in `shared/severity-rubric.md`. Apply the nits guide in
`shared/nits-guide.md`. Set `"reviewer": "cli-ux"`.
