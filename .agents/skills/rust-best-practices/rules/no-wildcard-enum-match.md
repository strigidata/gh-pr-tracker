---
title: Match Enums Exhaustively, Not with a Wildcard
impact: CRITICAL
tags: type-safety, enums, exhaustiveness, illegal-states
---

## Match Enums Exhaustively, Not with a Wildcard

A `_ =>` arm over an enum turns every future variant into a silent fallthrough. The value of modelling a domain as an enum is that adding a case breaks the build at every site that must decide what to do; a wildcard throws that away and converts a compile error into a wrong answer at runtime. List the variants instead, and let the compiler produce the migration checklist.

`clippy::wildcard_enum_match_arm` is a restriction lint, so it is not enabled by `clippy::all`, `pedantic`, or `nursery`. Turn it on explicitly.

**Incorrect (a new variant silently takes the default path):**

```rust
match operation {
    Operation::Buy => record_acquisition(),
    Operation::Sell => record_disposal(),
    // Operation::Swap, added later, lands here and is quietly ignored.
    _ => {}
}
```

**Correct (every variant is named):**

```rust
match operation {
    Operation::Buy => record_acquisition(),
    Operation::Sell => record_disposal(),
    Operation::Swap => record_exchange(),
}
```

When a genuine catch-all is required, bind it explicitly (`other => ...`) and document why the remaining variants share behaviour, or mark the enum `#[non_exhaustive]` so the wildcard is a deliberate part of the contract rather than an oversight.
