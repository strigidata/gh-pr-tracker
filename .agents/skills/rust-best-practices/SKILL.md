---
name: rust-best-practices
description: Idiomatic Rust best practices for code that must be safe, fast, and maintainable. Use when writing or reviewing any Rust code — covers error handling, ownership and borrowing, public API design, async patterns, and testing discipline.
metadata:
  priority: 5
  docs:
    - "https://doc.rust-lang.org/book/"
    - "https://rust-lang.github.io/api-guidelines/"
    - "https://doc.rust-lang.org/std/"
  pathPatterns:
    - "**/*.rs"
    - "**/Cargo.toml"
    - "**/Cargo.lock"
  bashPatterns:
    - '\bcargo\s+(build|test|clippy|fmt|run|check)\b'
    - '\brustc\b'
  importPatterns: []
  promptSignals:
    phrases:
      - "rust"
      - "cargo"
      - "ownership"
      - "borrow checker"
      - "newtype"
      - "illegal state"
    allOf: []
    anyOf:
      - "rustdoc"
      - "rust idioms"
      - "unwrap"
      - "result type"
      - "primitive obsession"
      - "unrepresentable"
      - "sum type"
      - "typestate"
      - "phantom data"
    noneOf: []
    minScore: 4
validate: []
retrieval:
  aliases:
    - rust review
    - rust patterns
    - cargo quality
    - rust hygiene
    - newtype pattern
    - make illegal states unrepresentable
  intents:
    - review rust code
    - improve rust safety
    - audit rust error handling
    - design rust public api
    - model a domain type
    - remove primitive obsession
    - make invalid states unrepresentable
  entities:
    - rust
    - result
    - ownership
    - unwrap
    - panic
    - tokio
    - cargo
    - newtype
    - enum
    - typestate
    - phantomdata
chainTo: []

---

# Rust Best Practices

Project-agnostic idiomatic Rust rules for code that must be safe, fast, and maintainable. Contains 21 rules across 6 categories. Each rule includes incorrect vs. correct examples to guide automated refactoring, code generation, and code review.

The governing principle is that **the type system should make illegal states unrepresentable**. Rust's newtypes, enums, and typestate all compile away, so a type that cannot express a wrong value is strictly better than a runtime check that rejects one. Every guard the compiler absorbs is a bug class removed, an error variant retired, and a test you no longer need.

## When to Apply

Reference these guidelines when:
- Writing new Rust modules, functions, types, or traits
- Reviewing Rust code for panics, unsound casts, or error-handling regressions
- Designing public APIs (functions, structs, enums, traits) consumed by other crates
- Working in async code paths (Tokio, async-std) where blocking or unbounded spawns can stall the runtime
- Adding or maintaining unit and integration tests for a Rust crate

## Rule Categories

| Category | Impact | Rules |
|----------|--------|-------|
| Illegal States | CRITICAL | `newtype-for-domain-values`, `enum-binds-tag-to-payload`, `no-wildcard-enum-match`, `no-bool-params`, `try-from-at-boundaries`, `typestate-for-lifecycle` |
| Error Handling | CRITICAL | `no-unwrap-in-prod`, `no-panic-in-prod`, `result-and-error-types`, `error-context-with-question-mark` |
| Ownership & Borrowing | HIGH | `prefer-borrow-over-clone`, `prefer-slice-over-vec-param`, `prefer-into-iter` |
| Public API Design | HIGH | `rustdoc-on-public-items`, `derive-debug-publictypes`, `module-visibility-discipline` |
| Async & Concurrency | HIGH | `tokio-runtime-discipline`, `mutex-vs-rwlock-vs-channel` |
| Code Quality & Testing | MEDIUM | `function-size-limit`, `iterators-over-loops`, `cfg-test-and-tests-module` |

## Quick Reference

### 1. Illegal States (CRITICAL)

- `newtype-for-domain-values` — Wrap domain primitives in newtypes with private fields and validating constructors. `UserId` and `OrderId` must not both be `u64`.
- `enum-binds-tag-to-payload` — Never carry a discriminant beside the data it describes. An enum variant owns its payload, so the two cannot disagree.
- `no-wildcard-enum-match` — No `_ =>` over an enum. Enable `clippy::wildcard_enum_match_arm`; it is a restriction lint, so `clippy::all`, `pedantic`, and `nursery` do not cover it.
- `no-bool-params` — Replace `bool` parameters and category-encoding `bool` fields with two-variant enums. `process(tx, true, false)` is unreadable and silently swappable.
- `try-from-at-boundaries` — Generated protobuf and serde types describe a wire format, not a domain. Convert once with `TryFrom` and keep them out of every signature past the boundary.
- `typestate-for-lifecycle` — Operations legal only in some lifecycle states belong to types that exist only in those states. `PhantomData` is erased, so this is free.

### 2. Error Handling (CRITICAL)

- `no-unwrap-in-prod` — `.unwrap()` and `.expect()` on `Result`/`Option` outside test code are panic-on-error paths. Prefer `?`, `match`, `ok_or(...)`, `unwrap_or_else(...)`. Allow `.unwrap()` only when documented infallible with a comment.
- `no-panic-in-prod` — `panic!()`, `todo!()`, `unimplemented!()`, `unreachable!()` in production paths abort the process. Use `Result` with a proper error type. `unreachable!()` is acceptable only when the compiler can't prove exhaustiveness.
- `result-and-error-types` — Use `Result<T, E>` with named error types — `thiserror` for libraries, `anyhow` for binaries. Don't return `Result<T, Box<dyn Error>>` from library APIs.
- `error-context-with-question-mark` — Use `?` to propagate errors, not `.unwrap()` followed by manual checks. Wrap with `.map_err(|e| MyError::Foo { source: e })` or `anyhow::Context` to add context.

### 3. Ownership & Borrowing (HIGH)

- `prefer-borrow-over-clone` — Take `&T` or `&str` when you don't need ownership. Avoid `.to_string()` / `.clone()` when a borrow would suffice. Cloning is a real allocation cost.
- `prefer-slice-over-vec-param` — Take `&[T]` not `&Vec<T>` for parameters. Slices are more flexible (accept arrays and `Vec`) and don't force allocation.
- `prefer-into-iter` — When you own the data and don't need it after iteration, `into_iter()` avoids unnecessary clones.

### 4. Public API Design (HIGH)

- `rustdoc-on-public-items` — Every `pub` function, struct, enum, and trait should have `///` documentation describing intent (not just signature restatement). Include `/// # Examples` for short public APIs.
- `derive-debug-publictypes` — Public types should derive `Debug` (and usually `Clone`, `PartialEq`) unless there's a deliberate reason not to. Helps debugging and testing.
- `module-visibility-discipline` — Default to private (`fn foo`). Use `pub(crate)` for crate-internal items, `pub` for the public API. Avoid `pub use` re-exports that obscure where an item is defined.

### 5. Async & Concurrency (HIGH)

- `tokio-runtime-discipline` — Don't call `block_on` from within an async context. Don't spawn unbounded tasks. Use `tokio::select!` for racing futures, `tokio::join!` for parallelism.
- `mutex-vs-rwlock-vs-channel` — Prefer message-passing (channels) over shared state. When state must be shared, `RwLock` for read-heavy, `Mutex` for balanced. Avoid `std::sync::Mutex` in async code — use `tokio::sync::Mutex`.

### 6. Code Quality & Testing (MEDIUM)

- `function-size-limit` — Functions should stay under ~50 lines with a single responsibility. Extract pure helpers from long functions to make them testable.
- `iterators-over-loops` — Prefer iterator chains (`.iter().filter().map().collect()`) over hand-written `for` loops where equivalent. They are more amenable to optimization and easier to compose.
- `cfg-test-and-tests-module` — Place unit tests in `#[cfg(test)] mod tests { ... }` at the bottom of the file under test. Integration tests live in `tests/`. Prefer `assert_eq!` / `assert_matches!` over `assert!` for actionable failure messages.

## How to Use

Read individual rule files for detailed explanations and code examples:

```
rules/no-unwrap-in-prod.md
rules/result-and-error-types.md
rules/tokio-runtime-discipline.md
```

Each rule file contains:
- A short YAML frontmatter (`title`, `impact`, `tags`)
- A one-paragraph explanation of why the rule matters
- An INCORRECT example with a brief note on what's wrong
- A CORRECT example with the fix
- One sentence on why the correct form is preferred

## Full Compiled Document

For the complete guide with every rule expanded inline, see the
skill-local compiled reference: [`./AGENTS.md`](./AGENTS.md)
(`.agents/skills/rust-best-practices/AGENTS.md` — not the repo-root
`AGENTS.md`, which is the project's conventions file).
