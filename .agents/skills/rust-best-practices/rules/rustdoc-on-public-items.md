---
title: Rustdoc on Every Public Item
impact: HIGH
tags: documentation, rustdoc, public-api, examples
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
