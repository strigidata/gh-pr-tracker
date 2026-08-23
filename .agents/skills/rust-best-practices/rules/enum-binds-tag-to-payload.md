---
title: Bind the Tag to Its Payload
impact: CRITICAL
tags: type-safety, enums, sum-types, illegal-states
---

## Bind the Tag to Its Payload

Carrying a discriminant next to the data it describes lets the two disagree. A `kind` field passed beside a payload, or a struct with several `Option` fields where exactly one should be set, permits combinations that have no meaning, and every consumer must then trust that the producer paired them correctly. A Rust enum binds each tag to exactly the data that tag implies, so the compiler enforces the pairing.

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

Wire formats such as protobuf often cannot express this, which is fine: convert once at the boundary and keep the enum as the internal type, so the mismatched pairing is unrepresentable everywhere the business logic runs.
