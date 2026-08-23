---
title: Prefer into_iter When You Own the Data
impact: MEDIUM
tags: iterators, ownership, clone, into_iter, performance
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
