---
title: Derive Debug (and Usually Clone, PartialEq) on Public Types
impact: MEDIUM
tags: public-api, derive, debug, clone, partialeq, testing
---

## Derive Debug (and Usually Clone, PartialEq) on Public Types

`Debug` is the lowest-friction way for callers to log, `dbg!`, or `assert_eq!` a value — omitting it forces downstream crates to wrap your type just to print it. `Clone` and `PartialEq` are nearly as common in tests. Derive them by default on public types; only opt out when there's a deliberate reason (e.g. a type owns a non-cloneable handle, or contains secrets that must not appear in logs — in which case implement `Debug` by hand and redact).

**Incorrect (no derives — caller can't log, can't assert equality):**

```rust
pub struct UserId(u64);

pub enum Plan {
    Free,
    Pro,
    Enterprise,
}
```

**Correct (default-derive the standard traits):**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UserId(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    Free,
    Pro,
    Enterprise,
}

// Secrets: implement Debug manually to redact.
pub struct ApiKey(String);

impl std::fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiKey(***)")
    }
}
```

Default derives make types ergonomic in logs and tests; the secret case shows when a hand-written `Debug` earns its keep.
