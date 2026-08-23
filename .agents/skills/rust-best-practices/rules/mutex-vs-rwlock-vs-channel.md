---
title: Choose Channels, RwLock, or Mutex Deliberately
impact: HIGH
tags: concurrency, async, mutex, rwlock, channels, tokio
---

## Choose Channels, RwLock, or Mutex Deliberately

Shared mutable state is the #1 source of concurrency bugs. Prefer message-passing (`tokio::sync::mpsc`, `crossbeam_channel`) — it eliminates locking entirely and makes ownership flow explicit. When shared state really is the right model, pick the cheapest lock for the access pattern: `RwLock` for read-heavy workloads, `Mutex` for write-heavy or roughly balanced. In async code, never hold a `std::sync::Mutex` across an `.await` — it blocks the worker thread; use `tokio::sync::Mutex` (or restructure to drop the guard before awaiting).

**Incorrect (std Mutex held across await — blocks the runtime):**

```rust
use std::sync::Mutex;

async fn update_cache(cache: &Mutex<HashMap<u64, Entry>>, id: u64) {
    let mut guard = cache.lock().unwrap();   // PoisonError swallowed by unwrap.
    let entry = fetch_entry(id).await;        // Holds the std Mutex across await!
    guard.insert(id, entry);
}
```

**Correct (channel-based actor — no shared lock at all):**

```rust
use tokio::sync::{mpsc, oneshot};

enum CacheMsg {
    Insert(u64, Entry),
    Get(u64, oneshot::Sender<Option<Entry>>),
}

async fn cache_actor(mut rx: mpsc::Receiver<CacheMsg>) {
    let mut cache: HashMap<u64, Entry> = HashMap::new();
    while let Some(msg) = rx.recv().await {
        match msg {
            CacheMsg::Insert(id, entry) => { cache.insert(id, entry); }
            CacheMsg::Get(id, reply) => { let _ = reply.send(cache.get(&id).cloned()); }
        }
    }
}
```

**Acceptable fallback (async-aware lock if you must share state):**

```rust
use tokio::sync::RwLock;

async fn read_entry(cache: &RwLock<HashMap<u64, Entry>>, id: u64) -> Option<Entry> {
    let guard = cache.read().await;
    guard.get(&id).cloned()
}
```

Message passing makes the data flow visible at the type level; when you do need a lock, picking an async-aware one keeps the runtime responsive.
