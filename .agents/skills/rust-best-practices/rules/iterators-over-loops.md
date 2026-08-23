---
title: Prefer Iterator Chains Over Hand-Written for Loops
impact: MEDIUM
tags: iterators, functional, idiomatic, performance
---

## Prefer Iterator Chains Over Hand-Written for Loops

Iterator adaptors (`.filter`, `.map`, `.fold`, `.flat_map`, `.collect`) make the data flow explicit and compose cleanly. They also give the optimizer a better shot at vectorizing or eliminating intermediate allocations — and they avoid common index-loop bugs (off-by-one, mutating the collection while iterating). Reach for a plain `for` loop when the body has side effects that don't fit a `for_each` (early return on error, complex control flow) or when an iterator chain genuinely hurts readability.

**Incorrect (manual loop with mutable accumulator and index bookkeeping):**

```rust
fn active_emails(users: &[User]) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for i in 0..users.len() {
        let user = &users[i];
        if user.is_active {
            result.push(user.email.to_lowercase());
        }
    }
    result
}
```

**Correct (iterator chain — declarative and allocation-friendly):**

```rust
fn active_emails(users: &[User]) -> Vec<String> {
    users
        .iter()
        .filter(|user| user.is_active)
        .map(|user| user.email.to_lowercase())
        .collect()
}
```

Iterator chains read like the spec ("active users, lowercased emails") and let the compiler fuse the passes into a single iteration.
