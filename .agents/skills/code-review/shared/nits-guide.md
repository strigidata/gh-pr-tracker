# Nits guide — when to suppress

The code-review pipeline optimizes for **signal**. A noisy report is
worse than a missed minor — the human or calling agent learns to ignore
the report. Four rules govern suppression:

## 1. Default-suppress rule

When in doubt, suppress. The cost of a noisy review is loss of signal
across the whole pipeline.

## 2. Branch-touched-it rule

If the branch touched a file and left it worse, that finding is
**Important** (not Nit). The "leave the codebase cleaner than it was
before" tenet wins. Examples:

- Function grew past ~50 lines on a file the branch edited
- RustDoc removed from a `pub` item
- New `.unwrap()` / `.expect()` introduced on a production path
- Dead code introduced
- A copy-paste duplicating an existing helper

Quote the tenet (`AGENTS.md` § "Working Style") in the finding's
`references` field.

## 3. Lint-equivalent rule

Anything `cargo fmt`, `cargo clippy`, `cargo check`, or `actionlint`
would catch is **suppressed**. CI handles it; reviewer noise wastes
signal. Examples to skip:

- Missing or unused imports, type errors
- Formatting (line length, brace placement, import order)
- Trivial naming (`foo` vs `Foo` for a type)
- Clippy lints already enabled in the workspace lint table

## 4. Out-of-diff rule

Issues on lines the branch did **not** touch are suppressed, unless
rule 2 applies (i.e., the branch did touch the file and made it worse
in a way that surfaces on a non-touched line — rare but legitimate).
