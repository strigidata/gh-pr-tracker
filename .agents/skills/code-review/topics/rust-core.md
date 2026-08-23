---
delegates:
  - rust-best-practices
---

# rust-core

## Role

You are reviewing Rust source in this repository — the library crate,
the CLI binary, and any supporting crates. You enforce the project's
governing principle: **the type system must make illegal states
unrepresentable**. A type that cannot express a wrong value beats a
runtime check that rejects one.

You do NOT lint or format — `cargo fmt` and `cargo clippy` handle that
and CI enforces them.

## Delegates to

Load the `rust-best-practices` skill before flagging any generic Rust
concern. That skill is the canonical source for the 21 rules across
Illegal States, Error Handling, Ownership & Borrowing, Public API
Design, Async & Concurrency, and Code Quality & Testing. Cite the rule
id (e.g. `newtype-for-domain-values`) in `references`. This playbook
covers only what is specific to *this* project and does not restate
those rules.

## Scope

- `crates/**/*.rs`, `src/**/*.rs`, `examples/**/*.rs`, `build.rs`
- `Cargo.toml`, `Cargo.lock`, workspace lint tables

## What to check

**Illegal states (highest priority):**

- A domain value carried as a bare primitive where a newtype exists or
  should: PR number, repo owner/name, node id, review-thread id, actor
  login, ETag, cursor. Two `u64`s that mean different things must not
  be interchangeable. **Important**, or **Critical** if the diff
  actually swaps two of them at a call site.
- A struct that pairs a discriminant with a payload the discriminant
  describes (e.g. a `kind: String` next to `Option<T>` fields where
  only some combinations are legal). Use an enum. **Important**.
- `_ =>` wildcard added over a domain enum (PR state, check
  conclusion, review decision, merge-state status). GitHub adds enum
  values; a wildcard silently absorbs them and renders them wrong.
  **Important** — enumerate the variants and add an explicit
  `Unknown(String)` arm if forward-compat is needed.
- A `bool` parameter or a `bool` field that encodes a category.
  **Important**.
- Wire types (serde/GraphQL/REST response structs) used past the
  boundary. Convert once with `TryFrom` into a domain type and keep the
  wire shape out of every downstream signature. **Important**.

**Error handling:**

- `.unwrap()` / `.expect()` on a `Result` or `Option` outside `#[cfg(test)]`.
  **Critical**, unless a comment documents why it is infallible.
- `panic!` / `todo!` / `unimplemented!` on a reachable path. **Critical**.
- `Result<T, Box<dyn Error>>` returned from the library crate's public
  API. **Important** — the library uses `thiserror`; the binary uses
  `anyhow`.
- An error propagated with `?` that loses the context needed to act on
  it (which repo, which PR, which request). **Important**.

**Async and concurrency:**

- `block_on`, `std::thread::sleep`, blocking file/network I/O, or a
  long CPU loop inside an async context. **Critical**.
- `tokio::spawn` inside a repeating interval or a per-item loop with no
  `JoinSet`, semaphore, or other bound — task count grows with every
  tick. **Critical**.
- `std::sync::Mutex` held across an `.await`. **Critical**.
- Shared mutable state where a channel would be simpler. **Suggestion**.
- A background task whose `JoinHandle` is dropped, so its panic or
  error is never observed. **Critical** — a silently dead poll task
  leaves the table rendering stale data forever.

**Cargo and dependencies:**

- A new dependency that duplicates one already in the tree (a second
  HTTP client, a second async runtime, a second TUI crate, a second
  error library). **Important** — name the existing one.
- A new dependency added with default features when the crate only
  needs a subset, on a CLI where binary size and build time matter.
  **Suggestion**.
- A crate added to the workspace without repeating the workspace lint
  table, if the project uses per-crate lint tables. **Important**.

## What NOT to flag

- Anything `cargo fmt` or `cargo clippy` catches — CI handles it.
- Pre-existing patterns in files the branch did not touch.
- Generic Rust idiom advice already covered verbatim by
  `rust-best-practices` — cite the rule instead of restating it.
- Dependency-version preferences without a concrete compatibility or
  security reason.

## External references (optional)

- `rust-best-practices` skill (delegated; canonical for generic rules)
- `AGENTS.md` § Code Standards

## Output contract

Return one JSON object per the schema in
`.agents/skills/code-review/shared/output-schema.md`. Use the severity
rubric in `shared/severity-rubric.md`. Apply the nits guide in
`shared/nits-guide.md`. Set `"reviewer": "rust-core"`.
