---
title: Default to Private; Use pub(crate) for Internal Items
impact: MEDIUM
tags: public-api, visibility, modules, encapsulation
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
