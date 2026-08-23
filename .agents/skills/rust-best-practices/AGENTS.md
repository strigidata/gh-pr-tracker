# Rust Best Practices — Compiled Reference

Project-agnostic idiomatic Rust rules, compiled from `SKILL.md` and `rules/*.md`. Use this file when you want every rule inline in one place; use individual `rules/*.md` files for focused reference.

See `SKILL.md` for the category overview, quick-reference table, and trigger metadata.

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

---


## Prefer Borrows Over Clones

Every `.clone()` on a `String`, `Vec`, `HashMap`, or other heap-backed type is a real allocation (and often a memcpy). When the function only needs to read the data, take `&T` or `&str` and let the caller keep ownership. Reach for `.clone()` only when you genuinely need a second independent owner — for example, before moving the value into a spawned task that outlives the caller.

**Incorrect (clones a `String` the function only reads):**

```rust
fn greet(name: String) -> String {
    format!("Hello, {name}!")
}

let user_name = String::from("Ada");
let greeting = greet(user_name.clone()); // clone just to keep `user_name` usable
println!("{user_name}");
```

**Correct (borrow with `&str`; no clone, no allocation cost at the call site):**

```rust
fn greet(name: &str) -> String {
    format!("Hello, {name}!")
}

let user_name = String::from("Ada");
let greeting = greet(&user_name);
println!("{user_name}");
```

Borrowing avoids both an allocation and a copy, and it documents that the function does not retain or mutate the value.

---


## Prefer &[T] Over &Vec<T> in Function Parameters

`&Vec<T>` constrains the caller to hand you a `Vec` specifically. `&[T]` accepts a `Vec<T>`, a fixed-size array, a `Box<[T]>`, or a sub-slice — and conveys that you only need to read the elements, not the `Vec`'s capacity or growth machinery. The same applies to `&str` versus `&String` for string parameters.

**Incorrect (caller must own a `Vec`, even when they have an array slice):**

```rust
fn sum(values: &Vec<i64>) -> i64 {
    values.iter().sum()
}

let total = sum(&vec![1, 2, 3]);
// Caller with `[i64; 3]` must `.to_vec()` first — unnecessary allocation.
```

**Correct (slice parameter accepts arrays, vecs, and sub-slices):**

```rust
fn sum(values: &[i64]) -> i64 {
    values.iter().sum()
}

let total_vec = sum(&vec![1, 2, 3]);
let total_arr = sum(&[1, 2, 3]);
let total_part = sum(&total_vec_data[1..]);
```

Slice parameters compose better with iterators, arrays, and zero-copy parsers — and they're a stable contract that doesn't tie callers to one collection type.

---


## Prefer into_iter When You Own the Data

`iter()` yields `&T`; `iter_mut()` yields `&mut T`; `into_iter()` consumes the collection and yields `T` by value. When you own the collection and don't need it again, `into_iter()` avoids the `.clone()` that would otherwise be needed to convert each `&T` into an owned `T`.

**Incorrect (iter then clone — one allocation per element):**

```rust
fn uppercase_all(names: Vec<String>) -> Vec<String> {
    names.iter().map(|n| n.clone().to_uppercase()).collect()
}
```

**Correct (into_iter consumes and yields owned `String`s — no clone):**

```rust
fn uppercase_all(names: Vec<String>) -> Vec<String> {
    names.into_iter().map(|n| n.to_uppercase()).collect()
}
```

Consuming iteration moves each element out of the source collection, which is both faster and clearer about the data's lifecycle.

---


## Rustdoc on Every Public Item

Every `pub fn`, `pub struct`, `pub enum`, and `pub trait` should carry a `///` comment that explains *intent* — what it's for and when to use it — not just a restatement of the signature. For short public APIs, include a `# Examples` block; runnable examples double as compile-time tests via `cargo test --doc`.

**Incorrect (signature-restating or absent docs):**

```rust
/// Gets the user.
pub fn get_user(id: u64) -> Option<User> { /* ... */ }

pub struct ApiClient { /* ... */ }
```

**Correct (intent, behaviour, and a usage example):**

```rust
/// Look up a user by their numeric id.
///
/// Returns `None` if no user with that id exists. Returns `Some(user)`
/// even for soft-deleted users — use [`User::is_active`] to filter.
///
/// # Examples
///
/// ```
/// # use mycrate::{get_user, UserId};
/// let user = get_user(UserId(42));
/// assert!(user.is_some());
/// ```
pub fn get_user(id: UserId) -> Option<User> { /* ... */ }

/// HTTP client for the public REST API.
///
/// Cheap to clone — internally wraps an `Arc` over the underlying
/// connection pool, so prefer cloning over re-constructing.
pub struct ApiClient { /* ... */ }
```

Intent-level docs make the API discoverable in `cargo doc` output and prevent the next maintainer from inferring behaviour from the body.

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

---


## Default to Private; Use pub(crate) for Internal Items

Every `pub` item is a commitment — a change to its signature is a breaking change for downstream crates. Start with no visibility (`fn foo`), elevate to `pub(crate)` when another module in the same crate needs it, and only mark `pub` when the item is part of the crate's deliberate public surface. Avoid `pub use` chains that re-export the same item from several modules — they hide where the item is defined and make refactors painful.

**Incorrect (everything is `pub`; helpers leak into the public API):**

```rust
// src/lib.rs
pub mod parser;

// src/parser.rs
pub fn parse(input: &str) -> Ast { /* ... */ }
pub fn normalize_whitespace(input: &str) -> String { /* helper */ }
pub struct ParserScratchpad { /* internal */ }
```

**Correct (publish the API; keep helpers crate-private):**

```rust
// src/lib.rs
mod parser;

pub use parser::{parse, Ast};

// src/parser.rs
pub fn parse(input: &str) -> Ast { /* ... */ }

pub(crate) fn normalize_whitespace(input: &str) -> String { /* helper */ }

pub(crate) struct ParserScratchpad { /* internal */ }
```

Narrow visibility shrinks the API surface area, makes the crate's intent obvious, and protects refactors from accidental breaking changes.

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

---


## Keep Functions Under ~50 Lines and Single-Purpose

Long functions hide bugs. Once a function exceeds ~50 lines or does more than one thing, individual steps stop being testable in isolation and reviewers stop reading carefully. The cure is almost always to extract pure helpers — small functions that take inputs, return outputs, and have no side effects — leaving the original function as a thin orchestrator.

**Incorrect (one 60-line function doing parse, validate, normalize, persist):**

```rust
pub fn import_user(raw: &str, db: &Db) -> Result<UserId, ImportError> {
    let mut parts = raw.split(',');
    let name = parts.next().ok_or(ImportError::MissingName)?.trim().to_string();
    let email_raw = parts.next().ok_or(ImportError::MissingEmail)?.trim();
    if !email_raw.contains('@') || email_raw.starts_with('@') {
        return Err(ImportError::InvalidEmail(email_raw.to_string()));
    }
    let email = email_raw.to_lowercase();
    let plan_raw = parts.next().unwrap_or("free").trim();
    let plan = match plan_raw {
        "free" => Plan::Free,
        "pro" => Plan::Pro,
        "ent" | "enterprise" => Plan::Enterprise,
        other => return Err(ImportError::UnknownPlan(other.to_string())),
    };
    let normalized_name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let row = NewUserRow { name: normalized_name, email, plan };
    let id = db.insert_user(row).map_err(ImportError::Db)?;
    Ok(id)
}
```

**Correct (orchestrator + small pure helpers, each independently testable):**

```rust
pub fn import_user(raw: &str, db: &Db) -> Result<UserId, ImportError> {
    let parsed = parse_user_csv(raw)?;
    let row = normalize_user(parsed)?;
    db.insert_user(row).map_err(ImportError::Db)
}

fn parse_user_csv(raw: &str) -> Result<RawUser, ImportError> { /* ~10 lines */ }

fn normalize_user(raw: RawUser) -> Result<NewUserRow, ImportError> { /* ~10 lines */ }
```

Short, single-purpose functions are easier to name, easier to unit-test, and dramatically easier for a reviewer to trust.

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

---


## Unit Tests in #[cfg(test)] mod tests, Integration Tests in tests/

Place unit tests at the bottom of the file they test, inside a `#[cfg(test)] mod tests { ... }` block. That lets them call private items, keeps test-only `use` statements out of the production build, and signals locality of test and code. Integration tests — those that exercise the crate from the outside — live as separate files in the crate's `tests/` directory. Prefer `assert_eq!` and `assert_matches!` over bare `assert!`; on failure they print both sides of the comparison, which makes triage 10× faster.

**Incorrect (tests in a sibling file, `assert!` that only prints "false"):**

```rust
// src/math.rs
pub fn add(a: i32, b: i32) -> i32 { a + b }

// src/math_test.rs  (not picked up by cargo, and can't see private items)
use crate::math::add;
#[test]
fn add_works() {
    assert!(add(2, 2) == 4);
    // On failure: "assertion failed: add(2, 2) == 4"  -- no values shown.
}
```

**Correct (co-located unit tests + assert_eq with both sides shown on failure):**

```rust
// src/math.rs
pub fn add(a: i32, b: i32) -> i32 { a + b }

fn checked_add(a: i32, b: i32) -> Option<i32> { a.checked_add(b) }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_sums_two_numbers() {
        assert_eq!(add(2, 2), 4);
    }

    #[test]
    fn checked_add_returns_none_on_overflow() {
        // assert_matches! (in matches crate or nightly std) prints the actual
        // variant on failure; in stable, fall back to a match + assert.
        assert!(matches!(checked_add(i32::MAX, 1), None));
    }
}
```

```rust
// tests/public_api.rs — integration test, only sees the crate's public surface.
use mycrate::add;

#[test]
fn add_is_exported() {
    assert_eq!(add(1, 2), 3);
}
```

Co-located unit tests stay in sync with the code; descriptive assertions cut failure-triage time.

---

---


## Wrap Domain Values in Newtypes

A function taking `(user_id: u64, order_id: u64)` accepts those arguments in either order, and a `String` field named `email` holds any text at all. A newtype with a private field and a validating constructor makes the value nominal and keeps its invariant, at zero runtime cost because the wrapper compiles away. Public fields defeat this: leave them private and expose an accessor, or callers will construct and mutate around the constructor.

**Incorrect (interchangeable identifiers and an unconstrained string):**

```rust
pub struct User {
    pub id: u64,
    pub email: String,
}

pub fn transfer(from: u64, to: u64, amount: u64) { /* ... */ }

// Compiles, and moves funds the wrong way.
transfer(to_id, from_id, amount);
```

**Correct (distinct types with checked construction):**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UserId(u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Email(String);

impl Email {
    /// Parses a raw string into a validated email address.
    pub fn parse(value: &str) -> Result<Self, EmailError> {
        if is_well_formed(value) {
            Ok(Self(value.to_owned()))
        } else {
            Err(EmailError::Malformed)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub fn transfer(from: UserId, to: UserId, amount: Amount) { /* ... */ }
```

The private field is the guarantee: outside the module there is no way to build an `Email` that skipped `parse`, so downstream code never re-checks.

---


## Bind the Tag to Its Payload

Carrying a discriminant next to the data it describes lets the two disagree. A `kind` field passed beside a payload, or a struct with several `Option` fields where exactly one should be set, permits combinations that have no meaning, and every consumer must trust that the producer paired them correctly. A Rust enum binds each tag to exactly the data that tag implies.

**Incorrect (the tag and the payload can disagree):**

```rust
pub enum RequestKind {
    Streaming,
    UpdateFmv,
}

// Nothing prevents RequestKind::UpdateFmv arriving with a streaming payload.
pub fn handle(kind: RequestKind, request: StreamingRequest) { /* ... */ }
```

**Correct (each variant carries its own data):**

```rust
pub enum Request {
    Streaming(StreamingRequest),
    UpdateFmv(UpdateFmvRequest),
}

pub fn handle(request: Request) {
    match request {
        Request::Streaming(req) => run_streaming(req),
        Request::UpdateFmv(req) => run_update_fmv(req),
    }
}
```

Wire formats such as protobuf often cannot express this, which is fine: convert once at the boundary and keep the enum as the internal type.

---


## Match Enums Exhaustively, Not with a Wildcard

A `_ =>` arm over an enum turns every future variant into a silent fallthrough. The value of modelling a domain as an enum is that adding a case breaks the build at every site that must decide what to do; a wildcard converts that compile error into a wrong answer at runtime. `clippy::wildcard_enum_match_arm` is a restriction lint, so it is not enabled by `clippy::all`, `pedantic`, or `nursery`. Turn it on explicitly.

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

When a genuine catch-all is required, bind it explicitly (`other => ...`) and document why the remaining variants share behaviour, or mark the enum `#[non_exhaustive]`.

---


## Replace Boolean Parameters with Enums

A `bool` parameter carries no meaning at the call site. `process(tx, true, false)` requires the reader to open the signature, and swapping two boolean arguments compiles silently. A two-variant enum names the intent and leaves room for a third case later. The same applies to `bool` struct fields that encode a category rather than a genuine yes-or-no fact.

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

Swapping the arguments now fails to compile, and a new variant makes exhaustive matching point at every site that must handle it.

---


## Convert Generated Types at the Boundary

Generated code from protobuf, serde, or a schema tool describes a wire format, not a domain. Those types make every field optional, use `String` where the domain has an enum, and cannot express which combinations are valid. Letting them travel into business logic spreads `unwrap` and re-validation everywhere.

**Incorrect (the wire type reaches the core, so every consumer re-checks):**

```rust
pub fn process(request: proto::RunRequest) -> Result<Report, Error> {
    // Repeated at every layer, and easy to forget in one of them.
    let portfolio_id = request.portfolio_id.ok_or(Error::MissingPortfolio)?;
    let mode = parse_mode(&request.mode)?;
    /* ... */
}
```

**Correct (one conversion, then a total domain type):**

```rust
pub struct RunRequest {
    portfolio_id: PortfolioId,
    mode: RunMode,
}

impl TryFrom<proto::RunRequest> for RunRequest {
    type Error = RequestError;

    fn try_from(wire: proto::RunRequest) -> Result<Self, Self::Error> {
        Ok(Self {
            portfolio_id: wire.portfolio_id.ok_or(RequestError::MissingPortfolio)?.into(),
            mode: RunMode::try_from(wire.mode.as_str())?,
        })
    }
}

pub fn process(request: RunRequest) -> Result<Report, Error> { /* ... */ }
```

Use `TryFrom` rather than `From` whenever the conversion can fail, and keep the generated type out of every signature past the boundary module.

---


## Encode Lifecycle State in the Type

When an entity passes through a lifecycle and some operations are legal only in some states, a `status` field defers the check to runtime. Every mutating method has to re-assert the state, and the one that forgets corrupts data meant to be final. Typestate moves the decision to the compiler.

**Incorrect (the guard is a convention each method must remember):**

```rust
impl Invoice {
    pub fn add_line(&mut self, line: Line) -> Result<(), InvoiceError> {
        // Omit this and an issued invoice is silently mutated.
        if self.status != Status::Draft {
            return Err(InvoiceError::Immutable);
        }
        self.lines.push(line);
        Ok(())
    }
}
```

**Correct (only a draft exposes the mutating method):**

```rust
pub struct Draft;
pub struct Issued;

pub struct Invoice<S> {
    lines: Vec<Line>,
    _state: PhantomData<S>,
}

impl Invoice<Draft> {
    pub fn add_line(&mut self, line: Line) {
        self.lines.push(line);
    }

    /// The only route to the immutable state.
    pub fn issue(self) -> Invoice<Issued> {
        Invoice { lines: self.lines, _state: PhantomData }
    }
}
```

`PhantomData` is erased at compile time, and `issue` taking `self` by value means the draft cannot be used again after the transition.
