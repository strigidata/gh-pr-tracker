---
title: Wrap Domain Values in Newtypes
impact: CRITICAL
tags: type-safety, newtype, domain-modelling, illegal-states
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

The private field is the guarantee: outside the module there is no way to build an `Email` that skipped `parse`, so downstream code never re-checks and never has to wonder.
