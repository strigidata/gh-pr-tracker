---
title: Convert Generated Types at the Boundary
impact: HIGH
tags: type-safety, boundaries, protobuf, serde, illegal-states
---

## Convert Generated Types at the Boundary

Generated code from protobuf, serde, or a schema tool describes a wire format, not a domain. Those types make every field optional, use `String` where the domain has an enum, and cannot express which combinations are valid. Letting them travel into business logic spreads `unwrap` and re-validation everywhere. Convert once, at the edge, with `TryFrom`, and keep the domain type as the only thing the rest of the code sees.

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

// Every field is present and well-typed by construction.
pub fn process(request: RunRequest) -> Result<Report, Error> { /* ... */ }
```

Use `TryFrom` rather than `From` whenever the conversion can fail, and keep the generated type out of every signature past the boundary module so the validation cannot be skipped.
