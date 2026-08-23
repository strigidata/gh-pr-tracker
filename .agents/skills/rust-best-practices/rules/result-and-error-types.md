---
title: Use Result with Named Error Types
impact: CRITICAL
tags: error-handling, result, thiserror, anyhow, error-types
---

## Use Result with Named Error Types

Library APIs should return `Result<T, E>` with a named error enum so callers can `match` on variants and so the type system documents what can go wrong. Use `thiserror` to derive `std::error::Error` cleanly in libraries; reserve `anyhow::Error` (or `Box<dyn Error>`) for binaries and tests where the caller will only log or exit. A library returning `Box<dyn Error>` forces every caller into a stringly-typed world.

**Incorrect (opaque error from a library function):**

```rust
pub fn load_user(id: u64) -> Result<User, Box<dyn std::error::Error>> {
    let row = db::fetch_user(id)?;
    Ok(User::try_from(row)?)
}
```

**Correct (typed error variants the caller can match on):**

```rust
#[derive(Debug, thiserror::Error)]
pub enum LoadUserError {
    #[error("user {0} not found")]
    NotFound(u64),
    #[error(transparent)]
    Db(#[from] db::Error),
    #[error("user row was malformed")]
    Malformed(#[source] UserParseError),
}

pub fn load_user(id: u64) -> Result<User, LoadUserError> {
    let row = db::fetch_user(id)?.ok_or(LoadUserError::NotFound(id))?;
    User::try_from(row).map_err(LoadUserError::Malformed)
}
```

Typed errors let callers branch on `NotFound` vs `Db` and survive future refactors that add new failure modes.
