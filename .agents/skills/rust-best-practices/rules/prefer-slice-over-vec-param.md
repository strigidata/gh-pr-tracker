---
title: Prefer &[T] Over &Vec<T> in Function Parameters
impact: MEDIUM
tags: ownership, slices, vec, api-design, deref-coercion
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
