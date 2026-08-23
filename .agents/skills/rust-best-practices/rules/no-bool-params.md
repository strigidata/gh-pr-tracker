---
title: Replace Boolean Parameters with Enums
impact: HIGH
tags: type-safety, enums, api-design, illegal-states
---

## Replace Boolean Parameters with Enums

A `bool` parameter carries no meaning at the call site. `process(tx, true, false)` requires the reader to open the signature, and swapping two boolean arguments compiles silently. A two-variant enum names the intent, makes the call self-describing, and leaves room to add a third case later without changing every caller's shape. The same applies to `bool` struct fields that encode a category rather than a genuine yes-or-no fact.

**Incorrect (unreadable at the call site, and the two flags can be swapped):**

```rust
pub fn post_entry(entry: &Entry, is_fiat: bool, dry_run: bool) { /* ... */ }

post_entry(&entry, true, false);
```

**Correct (each argument names itself):**

```rust
pub enum Denomination {
    Crypto,
    Fiat,
}

pub enum Execution {
    Commit,
    DryRun,
}

pub fn post_entry(entry: &Entry, denomination: Denomination, execution: Execution) { /* ... */ }

post_entry(&entry, Denomination::Fiat, Execution::Commit);
```

Swapping the arguments now fails to compile, and when a third denomination or execution mode appears, exhaustive matching points at every site that must handle it.
