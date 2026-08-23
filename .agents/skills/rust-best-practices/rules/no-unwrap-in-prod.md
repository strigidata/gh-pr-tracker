---
title: No unwrap or expect in Production Code
impact: CRITICAL
tags: error-handling, panic, unwrap, expect, result, option
---

## No unwrap or expect in Production Code

`.unwrap()` and `.expect()` on `Result` or `Option` panic the moment the value is `Err` / `None`. In a server, that aborts the request task (and on `std::sync::Mutex` can poison the lock); in a binary, it aborts the whole process. Reach for them only in tests, examples, or paths the compiler genuinely cannot prove infallible — and in that last case, leave a `// SAFETY` / `// unwrap: invariant is X` comment explaining why.

**Incorrect (panics on missing config or invalid number):**

```rust
fn port_from_env() -> u16 {
    let raw = std::env::var("PORT").unwrap();
    raw.parse::<u16>().expect("PORT must be a u16")
}
```

**Correct (propagate the error to the caller):**

```rust
fn port_from_env() -> Result<u16, ConfigError> {
    let raw = std::env::var("PORT").map_err(|_| ConfigError::Missing("PORT"))?;
    raw.parse::<u16>().map_err(|source| ConfigError::Invalid {
        key: "PORT",
        source,
    })
}
```

Propagating with `?` keeps panic-free control flow and lets callers decide how to surface the failure (log, retry, exit).
