# Severity rubric

Every finding emitted by a code-review reviewer **must** be classified
into one of these severities. Never emit Nits.

| Severity | Definition | Examples |
|----------|------------|----------|
| **Critical** | Bug, security risk, data-loss potential, broken public contract, or a panic on a reachable path — blocks merge. | `.unwrap()` on a `Result` from a network call; token or `Authorization` header written to stdout/logs; a `catch`-equivalent that swallows an error and returns an empty list; removed or renamed public API item without a semver bump; unbounded task spawn per polled PR; blocking I/O inside an async runtime. |
| **Important** | Pattern violation likely to cause future bugs, OR "left it worse" regression on a file the branch touched. | New `bool` parameter where an enum belongs; `_ =>` wildcard added over a domain enum; RustDoc removed from a `pub` item; function grew past ~50 lines on a file the branch edited; copy-paste duplicating an existing helper; dead code introduced; `&Vec<T>` parameter where `&[T]` works. |
| **Suggestion** | Concrete proposed change with file:line, not blocking. | "Extract this 35-line block into `render::column_widths`, used by both `table.rs` and `summary.rs`." |
| **Nit (suppress)** | Pure style / opinion / lint-equivalent — **NEVER emit**. | Formatting; import ordering; "rename `x` to `value`" without a functional reason; anything `cargo fmt` or `cargo clippy` already catches; pre-existing issues on lines the branch did not touch. |

See [`./nits-guide.md`](./nits-guide.md) for the suppression rules.

See [`./output-schema.md`](./output-schema.md) for the JSON shape each
reviewer must return.
