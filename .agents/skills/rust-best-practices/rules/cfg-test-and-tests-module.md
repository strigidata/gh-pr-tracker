---
title: Unit Tests in #[cfg(test)] mod tests, Integration Tests in tests/
impact: MEDIUM
tags: testing, cfg-test, integration-tests, assertions
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
