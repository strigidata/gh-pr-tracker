---
title: Propagate Errors with ? and Add Context
impact: HIGH
tags: error-handling, question-mark, map_err, anyhow, context
---

## Propagate Errors with ? and Add Context

`?` is the idiomatic way to short-circuit on `Err` — it's terser than `match`, doesn't risk a forgotten branch, and works with `From` impls to convert error types automatically. When the underlying error doesn't carry enough context (e.g. a generic `io::Error` from a file read), enrich it with `.map_err(..)` to a typed variant or with `anyhow::Context::with_context(..)` in a binary.

**Incorrect (unwrap, manual check, no context on the failure):**

```rust
fn load_config(path: &Path) -> Config {
    let bytes = std::fs::read(path).unwrap();
    if bytes.is_empty() {
        panic!("empty config");
    }
    serde_json::from_slice(&bytes).unwrap()
}
```

**Correct (propagate with `?`, attach the path so the failure is debuggable):**

```rust
use anyhow::Context;

fn load_config(path: &Path) -> anyhow::Result<Config> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("reading config at {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing config at {}", path.display()))
}
```

Context-rich errors turn "No such file or directory" into a log line that names the missing path and the operation that needed it.
