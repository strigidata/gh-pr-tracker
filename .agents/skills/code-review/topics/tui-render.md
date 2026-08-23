---
delegates:
  - rust-best-practices
---

# tui-render

## Role

You are reviewing the terminal rendering layer — the live table, its
layout, its refresh loop, colour, and terminal state management. A bug
here does not corrupt data; it corrupts the user's terminal, which they
notice immediately and forgive slowly.

You do NOT review GitHub data fetching (`github-api`) or argument
parsing (`cli-ux`).

## Scope

- Any module under a `render/`, `ui/`, `tui/`, `table/`, or `view/`
  directory
- Files importing a TUI or terminal crate (`ratatui`, `crossterm`,
  `termion`, `comfy-table`, `tabled`, `owo-colors`, `anstream`)
- The event/refresh loop that drives redraws

## What to check

**Terminal state — the unforgiving category:**

- Raw mode entered, or the alternate screen entered, without a restore
  on **every** exit path: normal return, `?` propagation, panic, and
  SIGINT/SIGTERM. **Critical** — a crash that leaves the terminal in
  raw mode forces the user to run `reset`. Require a guard type with a
  `Drop` impl plus a panic hook, not a cleanup call at the end of
  `main`.
- A panic hook installed that restores the terminal but then swallows
  the panic message. **Important** — restore, then print the payload.
- Cursor hidden without being shown again on exit. **Important**.

**Layout correctness:**

- Column width computed from `str::len()` or `chars().count()` instead
  of display width. **Important** — CJK titles, emoji in PR titles, and
  combining marks all break alignment. Use a unicode-width crate.
- Truncation that slices a `String` by byte index. **Critical** — a
  multi-byte boundary panics.
- Truncation that cuts inside an ANSI escape sequence, leaking a
  half-written escape into the terminal. **Critical**.
- Fixed column widths, or a layout that assumes a minimum terminal
  size, with no behavior defined for a narrow terminal. **Important** —
  name the width at which it breaks.
- No handling for a terminal resize (`SIGWINCH` / crossterm `Resize`)
  during a live session. **Important**.

**Live refresh:**

- A full-screen clear-and-redraw on every tick rather than a diffed
  update. **Important** if it produces visible flicker; **Suggestion**
  otherwise.
- Redraw driven only by a timer, so user input feels laggy, or driven
  only by input, so data updates never appear. Both sources must wake
  the loop. **Important**.
- Redraw rate coupled to the poll rate, so a 60s poll means a 60s input
  latency. **Critical**.
- A render function that performs I/O, allocates per cell, or calls the
  network. **Important** — render from an already-materialized view
  model.
- Data mutated by the poll task while the render task reads it, with no
  snapshot or lock discipline, producing a torn frame. **Important**.

**Output discipline and accessibility:**

- Colour or escape codes emitted when stdout is not a TTY, without
  honoring `NO_COLOR`, `CLICOLOR=0`, or a `--color` flag. **Critical**
  — this corrupts piped and redirected output.
- Meaning conveyed by colour alone (green = passing, red = failing)
  with no symbol or text fallback. **Important** — fails for
  colourblind users and for monochrome terminals.
- Colour chosen as a raw 24-bit value with no 16-colour fallback, on a
  tool expected to run over SSH and in basic terminals. **Suggestion**.
- Unicode box-drawing or symbols used with no ASCII fallback path.
  **Suggestion**.

**Empty and error states:**

- A rendered table with no visible distinction between "no PRs match"
  and "we could not reach GitHub". **Critical** — cite the same rule as
  `github-api`: failure must be representable in the view model, and
  the render layer must show it.
- Stale data rendered with no indication of its age after a failed
  refresh. **Important**.

## What NOT to flag

- Anything `cargo clippy` catches — CI handles it.
- Colour palette and glyph preferences, absent an accessibility or
  compatibility argument.
- Choice of TUI crate.
- Pre-existing rendering code the branch did not touch.

## External references (optional)

- `https://no-color.org/`
- `rust-best-practices` § Code Quality & Testing (function size, purity
  of the render path)

## Output contract

Return one JSON object per the schema in
`.agents/skills/code-review/shared/output-schema.md`. Use the severity
rubric in `shared/severity-rubric.md`. Apply the nits guide in
`shared/nits-guide.md`. Set `"reviewer": "tui-render"`.
