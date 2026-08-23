---
title: Encode Lifecycle State in the Type
impact: HIGH
tags: type-safety, typestate, state-modelling, illegal-states
---

## Encode Lifecycle State in the Type

When an entity passes through a lifecycle and some operations are legal only in some states, a `status` field defers the check to runtime. Every mutating method has to re-assert the state, and the one that forgets corrupts data meant to be final. Typestate moves the decision to the compiler: give each state its own type, and the illegal operation has no method to call.

**Incorrect (the guard is a convention each method must remember):**

```rust
pub struct Invoice {
    pub status: Status,
    pub lines: Vec<Line>,
}

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

impl Invoice<Issued> {
    pub fn lines(&self) -> &[Line] {
        &self.lines
    }
}
```

`PhantomData` is erased at compile time, so this costs nothing at runtime, and `issue` taking `self` by value means the draft cannot be used again after the transition.
