---
title: No panic, todo, unimplemented, or unreachable in Production
impact: CRITICAL
tags: error-handling, panic, todo, unimplemented, unreachable
---

## No panic, todo, unimplemented, or unreachable in Production

`panic!()`, `todo!()`, and `unimplemented!()` all abort the current thread — in async runtimes they take down the task; in a single-threaded binary they take down the process. `unreachable!()` is the same panic dressed in optimism, so reserve it for branches the compiler genuinely cannot prove dead (e.g. an exhaustive match over a parsed external value where the type system can't help). Everything else should be a `Result`.

**Incorrect (panic disguised as a placeholder):**

```rust
fn handle_event(event: Event) {
    match event.kind {
        EventKind::Created => store_created(event),
        EventKind::Updated => store_updated(event),
        EventKind::Deleted => todo!("delete branch not implemented yet"),
    }
}
```

**Correct (encode the unsupported case as an error):**

```rust
fn handle_event(event: Event) -> Result<(), HandlerError> {
    match event.kind {
        EventKind::Created => store_created(event),
        EventKind::Updated => store_updated(event),
        EventKind::Deleted => Err(HandlerError::Unsupported("delete")),
    }
}
```

Returning an error keeps the runtime alive and gives callers a chance to handle, log, or retry the unsupported case.
