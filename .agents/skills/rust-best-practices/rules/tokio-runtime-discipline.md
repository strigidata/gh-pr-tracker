---
title: Tokio Runtime Discipline — No block_on in Async, Bound Spawns
impact: HIGH
tags: async, tokio, block_on, spawn, select, join
---

## Tokio Runtime Discipline — No block_on in Async, Bound Spawns

Calling `Runtime::block_on` (or `futures::executor::block_on`) from inside an async function blocks the worker thread that should be polling other futures — in the worst case it deadlocks the runtime. Spawning unbounded tasks (one per incoming request, no backpressure) lets a slow downstream system OOM your process. Use `tokio::join!` / `tokio::select!` for in-task concurrency, and bound concurrency with `JoinSet`, a semaphore, or a worker pool.

**Incorrect (block_on inside async, unbounded spawn loop):**

```rust
async fn fan_out(items: Vec<Item>) {
    for item in items {
        // Spawns a task per item with no limit — easy DoS.
        tokio::spawn(async move {
            // Blocks the worker thread for the entire request.
            tokio::runtime::Handle::current().block_on(process(item));
        });
    }
}
```

**Correct (in-place async, bounded concurrency via JoinSet):**

```rust
use tokio::task::JoinSet;

async fn fan_out(items: Vec<Item>) -> Result<(), ProcessError> {
    let mut set = JoinSet::new();
    let mut iter = items.into_iter();

    // Keep at most 16 in flight at any time.
    for _ in 0..16 {
        if let Some(item) = iter.next() {
            set.spawn(process(item));
        }
    }
    while let Some(joined) = set.join_next().await {
        joined.map_err(ProcessError::Join)??;
        if let Some(item) = iter.next() {
            set.spawn(process(item));
        }
    }
    Ok(())
}
```

Awaiting instead of blocking keeps the runtime healthy, and bounded concurrency protects you from being knocked over by your own throughput.
