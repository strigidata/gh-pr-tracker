---
title: Prefer Borrows Over Clones
impact: HIGH
tags: ownership, borrowing, clone, allocation, performance
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
